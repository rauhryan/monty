"""Trusted host registry/journal; none of its management methods are guest exports."""

import json
import secrets
import sqlite3
from contextlib import contextmanager


def canonical(value):
    """Stable JSON identity; reject non-JSON/nonfinite payloads."""
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


class Denied(ValueError):
    """The requested identity or current authority does not permit dispatch."""


class Indeterminate(RuntimeError):
    """Intent exists without durable outcome; automatic redispatch is forbidden."""


class Crash(RuntimeError):
    """Fault injection at a durable boundary, modeling adapter process loss."""


class Journal:
    """SQLite is host-held, outside snapshot rollback; one connection per caller."""

    def __init__(self, path):
        self.db = sqlite3.connect(path, timeout=10, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute('PRAGMA synchronous=FULL')
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, tenant TEXT, cancelled INTEGER, reason TEXT);
            CREATE TABLE IF NOT EXISTS lineages(id TEXT PRIMARY KEY, run TEXT);
            CREATE TABLE IF NOT EXISTS caps(
                id TEXT PRIMARY KEY, run TEXT, tenant TEXT, op TEXT, resource TEXT,
                limit_value INTEGER, revoked INTEGER);
            CREATE TABLE IF NOT EXISTS cuts(
                id TEXT PRIMARY KEY, lineage TEXT, parent TEXT, edge TEXT, snapshot BLOB,
                backend TEXT, request TEXT, effect TEXT, UNIQUE(lineage,parent,edge));
            CREATE TABLE IF NOT EXISTS effects(
                id TEXT PRIMARY KEY, request TEXT, outcome TEXT);
        """)

    def close(self):
        """Release this adapter's database connection."""
        self.db.close()

    @contextmanager
    def transaction(self):
        """Serialize intent admission and authority mutations across connections."""
        self.db.execute('BEGIN IMMEDIATE')
        try:
            yield
            self.db.execute('COMMIT')
        except BaseException:
            self.db.execute('ROLLBACK')
            raise

    def create_run(self, tenant):
        """Trusted provisioning creates run and initial lineage, never a guest label."""
        run, lineage = secrets.token_hex(16), secrets.token_hex(16)
        with self.transaction():
            self.db.execute('INSERT INTO runs VALUES (?,?,0,NULL)', (run, tenant))
            self.db.execute('INSERT INTO lineages VALUES (?,?)', (lineage, run))
        return run, lineage

    def grant(self, run, op, resource, limit=1):
        """Pregrant one resource/operation; only release survives run cancellation."""
        if op not in ('reserve', 'charge', 'release') or type(limit) is not int or limit < 1:
            raise Denied('invalid grant')
        with self.transaction():
            owner = self.db.execute('SELECT * FROM runs WHERE id=?', (run,)).fetchone()
            if owner is None or owner['cancelled']:
                raise Denied('run unavailable')
            cap = secrets.token_hex(24)
            self.db.execute('INSERT INTO caps VALUES (?,?,?,?,?,?,0)', (cap, run, owner['tenant'], op, resource, limit))
        return cap

    def cancel(self, run, reason='host cancelled'):
        """Irreversible host cancellation; already admitted effects may finish."""
        with self.transaction():
            self.db.execute('UPDATE runs SET cancelled=1,reason=? WHERE id=? AND cancelled=0', (reason, run))

    def revoke(self, cap):
        """Revoke NEW admissions; recorded outcomes remain replayable."""
        with self.transaction():
            self.db.execute('UPDATE caps SET revoked=1 WHERE id=?', (cap,))

    def owner(self, tenant, run, lineage):
        """Validate caller context supplied by a trusted embedding, not guest JSON."""
        row = self.db.execute(
            'SELECT runs.* FROM runs JOIN lineages ON runs.id=lineages.run '
            'WHERE runs.id=? AND tenant=? AND lineages.id=?',
            (run, tenant, lineage),
        ).fetchone()
        if row is None:
            raise Denied('lineage ownership mismatch')
        return row

    def capture(self, tenant, run, lineage, state, parent='', edge='start', *, retained_wait=False):
        """Trusted ingestion; only a transport-confirmed retained wait keeps the parent effect."""
        encoded = canonical(state.request)
        with self.transaction():
            self.owner(tenant, run, lineage)
            prior = None
            if parent:
                prior = self.cut(tenant, run, lineage, parent)
            if retained_wait and (
                prior is None or edge != 'cancel' or prior['request'] != encoded or prior['backend'] != state.backend
            ):
                raise Denied('invalid retained wait transition')
            old = self.db.execute(
                'SELECT * FROM cuts WHERE lineage=? AND parent=? AND edge=?', (lineage, parent, edge)
            ).fetchone()
            if old:
                if old['request'] != encoded or old['backend'] != state.backend:
                    raise Denied('divergent replay requires explicit host fork grant')
                if prior and (old['effect'] == prior['effect']) != retained_wait:
                    raise Denied('divergent replay wait transition')
                return old['id']
            # Descriptor or snapshot equality cannot identify a newly emitted await.
            effect = prior['effect'] if retained_wait else secrets.token_hex(24)
            cut = secrets.token_hex(24)
            self.db.execute(
                'INSERT INTO cuts VALUES (?,?,?,?,?,?,?,?)',
                (cut, lineage, parent, edge, state.snapshot, state.backend, encoded, effect),
            )
            return cut

    def cut(self, tenant, run, lineage, checkpoint):
        """Load only host-held bytes; callers cannot substitute snapshots/descriptors."""
        self.owner(tenant, run, lineage)
        row = self.db.execute('SELECT * FROM cuts WHERE id=? AND lineage=?', (checkpoint, lineage)).fetchone()
        if row is None:
            raise Denied('checkpoint lineage mismatch')
        return row

    def fork(self, tenant, run, lineage, checkpoint):
        """Explicit trusted host grant for a new effectful branch of an active run."""
        with self.transaction():
            if self.owner(tenant, run, lineage)['cancelled']:
                raise Denied('cancelled run cannot fork')
            cut = self.cut(tenant, run, lineage, checkpoint)
            branch, new_cut = secrets.token_hex(16), secrets.token_hex(24)
            self.db.execute('INSERT INTO lineages VALUES (?,?)', (branch, run))
            self.db.execute(
                'INSERT INTO cuts VALUES (?,?,?,?,?,?,?,?)',
                (new_cut, branch, '', 'start', cut['snapshot'], cut['backend'], cut['request'], secrets.token_hex(24)),
            )
        return branch, new_cut

    def authorize(self, tenant, run, lineage, request):
        """Exact typed schema and per-operation resource constraint, no arbitrary dispatcher."""
        owner = self.owner(tenant, run, lineage)
        if not isinstance(request, dict) or set(request) != {'id', 'cap', 'op', 'args'}:
            raise Denied('invalid descriptor')
        if type(request['id']) is not int or type(request['cap']) is not str:
            raise Denied('invalid local label or handle')
        cap = self.db.execute('SELECT * FROM caps WHERE id=?', (request['cap'],)).fetchone()
        if cap is None or cap['run'] != run or cap['tenant'] != tenant or cap['revoked']:
            raise Denied('unknown, foreign or revoked capability')
        if request['op'] != cap['op'] or (owner['cancelled'] and cap['op'] != 'release'):
            raise Denied('operation unavailable')
        args = request['args']
        field = {'reserve': 'quantity', 'charge': 'amount', 'release': None}[cap['op']]
        if not isinstance(args, dict) or set(args) != ({'resource', field} if field else {'resource'}):
            raise Denied('invalid arguments')
        if args['resource'] != cap['resource']:
            raise Denied('wrong resource')
        if field and (type(args[field]) is not int or not 1 <= args[field] <= cap['limit_value']):
            raise Denied('constraint violation')

    def settle(self, tenant, run, lineage, checkpoint, request, service, fault=None):
        """Persist intent, dispatch once, persist outcome; pending submissions never dispatch."""
        encoded = canonical(request)
        with self.transaction():
            cut = self.cut(tenant, run, lineage, checkpoint)
            if cut['request'] != encoded:
                raise Denied('effect identity payload mismatch')
            effect = cut['effect']
            old = self.db.execute('SELECT * FROM effects WHERE id=?', (effect,)).fetchone()
            if old:
                if old['request'] != encoded:
                    raise Denied('effect identity collision')
                if old['outcome'] is None:
                    raise Indeterminate(effect)
                return json.loads(old['outcome'])
            self.authorize(tenant, run, lineage, request)
            self.db.execute('INSERT INTO effects VALUES (?,?,NULL)', (effect, encoded))
        if fault == 'before_dispatch':
            raise Crash(fault)
        outcome = service.apply(effect, request)
        if fault == 'after_dispatch':
            raise Crash(fault)
        with self.transaction():
            self.db.execute('UPDATE effects SET outcome=? WHERE id=?', (canonical(outcome), effect))
        if fault == 'after_completion':
            raise Crash(fault)
        return outcome

    def reconcile(self, tenant, run, lineage, checkpoint, service):
        """Read-only downstream lookup; absence is indeterminate, NEVER an implicit retry."""
        cut = self.cut(tenant, run, lineage, checkpoint)
        old = self.db.execute('SELECT * FROM effects WHERE id=?', (cut['effect'],)).fetchone()
        if old is None:
            raise Denied('no admitted intent')
        if old['outcome'] is not None:
            return json.loads(old['outcome'])
        outcome = service.lookup(cut['effect'], json.loads(old['request']))
        if outcome is None:
            raise Indeterminate(cut['effect'])
        with self.transaction():
            self.db.execute('UPDATE effects SET outcome=? WHERE id=?', (canonical(outcome), cut['effect']))
        return outcome


class FakeService:
    """Separate durable service with atomic idempotency-key/result storage; no real I/O."""

    def __init__(self, path):
        self.db = sqlite3.connect(path, timeout=10)
        self.db.execute('CREATE TABLE IF NOT EXISTS calls(id TEXT PRIMARY KEY, request TEXT, outcome TEXT)')
        self.db.commit()

    def close(self):
        """Release the fake downstream connection."""
        self.db.close()

    def lookup(self, key, request):
        """Reconcile a previously applied key without repeating external work."""
        row = self.db.execute('SELECT request,outcome FROM calls WHERE id=?', (key,)).fetchone()
        if row is None:
            return None
        if row[0] != canonical(request):
            raise Denied('downstream idempotency conflict')
        return json.loads(row[1])

    def apply(self, key, request):
        """Model atomic business mutation plus key/result persistence in a separate database."""
        with self.db:
            self.db.execute('BEGIN IMMEDIATE')
            old = self.lookup(key, request)
            if old is not None:
                return old
            if request['op'] == 'reserve':
                value = {'reservation': 'hold-1', 'total': 42}
            elif request['op'] == 'charge':
                value = {'receipt': key}
            elif request['op'] == 'release':
                value = {'released': request['args']['resource']}
            else:
                raise Denied('unsupported fake operation')
            self.db.execute('INSERT INTO calls VALUES (?,?,?)', (key, canonical(request), canonical(value)))
            return value

    def count(self, operation):
        """Count actual modeled mutations, not attempts or lookups."""
        return sum(json.loads(row[0])['op'] == operation for row in self.db.execute('SELECT request FROM calls'))
