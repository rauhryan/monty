"""Literal host-boundary regressions over genuine native and Wasm snapshots."""

import argparse
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace

from demo import provision
from host import Finished, Host, Sandbox, Suspended, invoke
from journal import Denied

ROOT = Path(__file__).resolve().parents[1]


def start(directory, host, handles, backend, body):
    """Retain the authored guest and genuine producer bytes; no fabricated VM state."""
    source, snapshot = directory / 'guest.py', directory / 'initial.bin'
    source.write_text(
        (ROOT / 'contracts.py').read_text()
        + '\n'
        + '\n'.join(f'{k} = {v!r}' for k, v in zip(('INVENTORY', 'PAYMENT', 'RELEASE'), handles, strict=True))
        + '\n'
        + body
        + '\njson.dumps(entry()._native1())\n'
    )
    view = invoke(Sandbox(backend).command, ['start', source, snapshot], host.evidence)
    assert set(view) == {'effect'}
    return host.journal.capture(*host.context, Suspended(snapshot.read_bytes(), view['effect'], backend))


def effect(host, cut):
    """Inspect the durable host identity independently of guest descriptors."""
    return host.journal.cut(*host.context, cut)['effect']


def record(host, terminal):
    """Retain requests, host identity and genuine snapshot hashes for independent review."""
    assert isinstance(terminal, Finished)
    return {
        'terminal': terminal.outcome,
        'counts': {op: host.service.count(op) for op in ('reserve', 'charge', 'release')},
        'cuts': [
            {
                'id': row['id'],
                'parent': row['parent'],
                'edge': row['edge'],
                'effect': row['effect'],
                'request': json.loads(row['request']),
                'snapshot_sha256': hashlib.sha256(row['snapshot']).hexdigest(),
            }
            for row in host.journal.db.execute('SELECT * FROM cuts')
        ],
        'start_evidence': host.evidence,
    }


def sequential(directory, host, handles, backend, revoke):
    """Identical sequential descriptors are distinct effects, including after revocation."""
    first = start(
        directory,
        host,
        handles,
        backend,
        """
async def entry():
    global LOCAL_LABEL
    first = await charge(1)
    LOCAL_LABEL = 0
    try:
        second = await charge(1)
    except ValueError as error:
        return {'first': first, 'denial': str(error)}
    return {'first': first, 'second': second}
""",
    )
    second = host.advance(first)
    assert host.state(first).request == host.state(second).request
    assert effect(host, first) != effect(host, second)
    assert host.advance(first) == second
    assert host.service.count('charge') == 1
    first_receipt = {'receipt': effect(host, first)}
    if revoke:
        host.journal.revoke(handles[1])
    terminal = host.advance(second)
    expected = (
        {'first': first_receipt, 'denial': 'unknown, foreign or revoked capability'}
        if revoke
        else {'first': first_receipt, 'second': {'receipt': effect(host, second)}}
    )
    assert terminal.outcome == expected
    assert host.service.count('charge') == (1 if revoke else 2)
    # Canonical replay after revocation still returns only the *first* recorded outcome.
    assert host.settle(first) == first_receipt
    assert host.advance(first) == second
    assert host.advance(second) == terminal
    return record(host, terminal)


def malformed(directory, host, handles, backend, descriptor, message):
    """Malformed descriptors raise catchable guest ValueError and unwind legitimate release."""
    first = start(
        directory,
        host,
        handles,
        backend,
        f"""
def bad():
    yield exchange(json.dumps({descriptor}))
async def entry():
    await reserve(1)
    try:
        await bad()
    except ValueError as error:
        return {{'denial': str(error)}}
    finally:
        await release()
    return {{'status': 'unexpected success'}}
""",
    )
    bad = host.advance(first)
    cleanup = host.advance(bad)
    assert host.state(cleanup).request['op'] == 'release'
    assert host.advance(bad) == cleanup
    assert host.service.count('charge') == host.service.count('release') == 0
    terminal = host.advance(cleanup)
    assert terminal.outcome == {'denial': message}
    assert host.service.count('reserve') == host.service.count('release') == 1
    assert host.service.count('charge') == 0
    assert host.advance(cleanup) == terminal
    assert host.service.count('release') == 1
    return record(host, terminal)


def retained(directory, host, handles, backend):
    """Repeated protected cancellation retains one effect even across host restart."""
    reserve = host.start(backend, handles)
    payment = host.advance(reserve)
    cleanup = host.advance(payment)
    first = host.advance(cleanup, cancel='first')
    second = host.advance(first, cancel='second')
    assert effect(host, cleanup) == effect(host, first) == effect(host, second)
    assert host.advance(cleanup, cancel='third') == first
    journal_type = type(host.journal)
    host.journal.close()
    host.journal = journal_type(directory / 'host.db')
    restored = Host(host.journal, host.service, *host.context)
    terminal = restored.advance(second)
    assert terminal.outcome['status'] == 'cancelled'
    assert terminal.outcome['reason'] == 'first'
    assert host.service.count('release') == 1
    assert restored.settle(cleanup) == {'released': 'hold-1'}
    return record(host, terminal)


def cancel_new(directory, host, handles, backend):
    """Cancellation can emit a NEW identical descriptor; equality cannot imply retention."""
    first = start(
        directory,
        host,
        handles,
        backend,
        """
async def entry():
    global LOCAL_LABEL
    try:
        await release()
    finally:
        LOCAL_LABEL = 0
        await release()
""",
    )
    cleanup = host.advance(first, cancel='stop')
    assert host.state(first).request == host.state(cleanup).request
    assert effect(host, first) != effect(host, cleanup)
    assert host.advance(first, cancel='again') == cleanup
    protected = host.advance(cleanup, cancel='repeat')
    assert effect(host, cleanup) == effect(host, protected)
    terminal = host.advance(protected)
    assert terminal.outcome['status'] == 'cancelled' and terminal.outcome['reason'] == 'stop'
    assert host.service.count('release') == 1
    return record(host, terminal)


def journal_transitions(host, handles):
    """Reject inconsistent privileged transition metadata without trusting snapshot equality."""
    request = {'id': 0, 'cap': handles[1], 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}}
    state = SimpleNamespace(snapshot=b'journal-only-never-loaded', request=request, backend='test')
    first = host.journal.capture(*host.context, state)
    second = host.journal.capture(*host.context, state, parent=first, edge='settled')
    assert effect(host, first) != effect(host, second)
    retained_cut = host.journal.capture(*host.context, state, parent=first, edge='cancel', retained_wait=True)
    assert effect(host, retained_cut) == effect(host, first)
    for kwargs, message in (
        ({'retained_wait': True}, 'invalid retained wait transition'),
        ({'parent': first, 'edge': 'settled', 'retained_wait': True}, 'invalid retained wait transition'),
        ({'parent': first, 'edge': 'cancel'}, 'divergent replay wait transition'),
    ):
        try:
            host.journal.capture(*host.context, state, **kwargs)
        except Denied as error:
            assert str(error) == message
        else:
            raise AssertionError('inconsistent transition accepted')
    return {'status': 'PASS'}


def main():
    """Run both real backends and retain fresh evidence without overwriting prior results."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    base = "{'id': LABEL, 'cap': PAYMENT, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}}"
    cases = {
        'missing-id': (
            "{'cap': PAYMENT, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}}",
            'invalid descriptor',
        ),
        'scalar': ('7', 'invalid descriptor'),
        'string': ("'bad'", 'invalid descriptor'),
        'list': ('[]', 'invalid descriptor'),
        'null': ('None', 'invalid descriptor'),
        **{
            f'id-{name}': (base.replace('LABEL', value), 'invalid local label or handle')
            for name, value in (
                ('null', 'None'),
                ('bool', 'True'),
                ('float', '1.5'),
                ('string', "'0'"),
                ('list', '[]'),
                ('object', '{}'),
            )
        },
        'forged-cap': (
            base.replace('LABEL', '0').replace('PAYMENT', "'forged'"),
            'unknown, foreign or revoked capability',
        ),
    }
    for backend in ('native', 'wasm'):
        for name in ('sequential', 'revoke-second', 'retained', 'cancel-new', *cases):
            directory = args.output / f'{backend}-{name}'
            directory.mkdir()
            host, handles = provision(directory)
            try:
                if name in ('sequential', 'revoke-second'):
                    result = sequential(directory, host, handles, backend, name == 'revoke-second')
                elif name == 'retained':
                    result = retained(directory, host, handles, backend)
                elif name == 'cancel-new':
                    result = cancel_new(directory, host, handles, backend)
                else:
                    result = malformed(directory, host, handles, backend, *cases[name])
                results[f'{backend}-{name}'] = result
                (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
                print(f'PASS {backend}-{name}', flush=True)
            finally:
                host.journal.close()
                host.service.close()
    directory = args.output / 'journal-transitions'
    directory.mkdir()
    host, handles = provision(directory)
    try:
        results['journal-transitions'] = journal_transitions(host, handles)
    finally:
        host.journal.close()
        host.service.close()
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print(f'PASS {len(results)} scenarios')


if __name__ == '__main__':
    main()
