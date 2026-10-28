"""Host-only checkpoint driver, reusing stepping resume and current native1 start."""

import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

from journal import Denied

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent))
from native1.adapter import invoke  # noqa: E402
from stepping.host import Finished, Reply, Sandbox, Suspended  # noqa: E402


class Host:
    """Embedding owns caller context; guests supply only constrained descriptors."""

    def __init__(self, journal, service, tenant, run, lineage):
        self.journal, self.service = journal, service
        self.context = (tenant, run, lineage)
        self.evidence = []

    def start(self, backend, handles, after_cleanup=False):
        """Compile supplied contracts and authored guest with explicit scoped grants."""
        sandbox = Sandbox(backend)
        with TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source, snapshot = directory / 'guest.py', directory / 'snapshot.bin'
            source.write_text(
                (ROOT / 'contracts.py').read_text()
                + '\n'
                + '\n'.join(
                    f'{name} = {value!r}'
                    for name, value in zip(('INVENTORY', 'PAYMENT', 'RELEASE'), handles, strict=True)
                )
                + f'\nAFTER_CLEANUP = {after_cleanup!r}\n'
                + (ROOT / 'guest.py').read_text()
                + '\njson.dumps(entry()._native1())\n'
            )
            view = invoke(sandbox.command, ['start', source, snapshot], self.evidence)
            state = Suspended(snapshot.read_bytes(), view['effect'], backend)
            return self.journal.capture(*self.context, state)

    def state(self, checkpoint):
        """Retrieve the authenticated-by-custody checkpoint; no caller bytes accepted."""
        cut = self.journal.cut(*self.context, checkpoint)
        return Suspended(cut['snapshot'], json.loads(cut['request']), cut['backend'])

    def settle(self, checkpoint, fault=None):
        """Submit exactly the stored descriptor with its host-assigned effect identity."""
        return self.journal.settle(*self.context, checkpoint, self.state(checkpoint).request, self.service, fault)

    def advance(self, checkpoint, cancel=None):
        """Recheck authority at settlement; denial enters the guest as an ordinary error."""
        state = self.state(checkpoint)
        if cancel is not None:
            self.journal.cancel(self.context[1], cancel)
            reason = self.journal.owner(*self.context)['reason']
            result = Sandbox(state.backend).cancel(state, reason)
            edge = 'cancel'
        else:
            try:
                reply = Reply(None, self.settle(checkpoint))
                edge = 'settled'
            except Denied as error:
                # Native/Wasm assert exactly one pending future and select its real ID.
                # Schema denial must reach that await even without a usable guest label.
                reply = Reply(None, error=str(error))
                edge = 'denied'
            owner = self.journal.owner(*self.context)
            result = Sandbox(state.backend).advance(
                state, reply, cancel=owner['reason'] if owner['cancelled'] else None
            )
        if isinstance(result, Finished):
            return result
        return self.journal.capture(
            *self.context, result, parent=checkpoint, edge=edge, retained_wait=result.retained_wait
        )
