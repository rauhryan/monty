"""Synchronous stepping facade over the existing native1 transport, not a scheduler."""

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Literal

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


@dataclass(frozen=True)
class Suspended:
    """Opaque trusted Monty bytes and the external request at this cut."""

    snapshot: bytes
    request: object
    backend: str
    retained_wait: bool = False


@dataclass(frozen=True)
class Finished:
    """Guest outcome after cleanup finishes; no resumable state remains."""

    outcome: dict


@dataclass(frozen=True)
class Reply:
    """One settlement; None routes to the transport's sole real pending future."""

    request_id: int | None
    value: object = None
    error: str | None = None


class Sandbox:
    """Every advance uses a fresh process; no live Python generator is retained."""

    def __init__(self, backend: Literal['native', 'wasm'] = 'native'):
        if backend not in ('native', 'wasm'):
            raise ValueError('backend must be native or wasm')
        self.backend = backend
        self.command = (
            [str(REPO / 'playground/coroutine-baseline/target/debug/native1')]
            if backend == 'native'
            else ['node', str(ROOT.parent / 'native1/wasm.mjs')]
        )

    def advance(
        self, state: Suspended | None = None, reply: Reply | None = None, *, cancel: str | None = None
    ) -> Suspended | Finished:
        """Run synchronously to the next external wait or completed outcome."""
        if state is None and (reply is not None or cancel is not None):
            raise ValueError('initial advance takes no input')
        if state is not None:
            if state.backend != self.backend:
                raise ValueError('use the producing backend for this scaffold')
            if (reply is None and cancel is None) or (
                reply is not None
                and reply.request_id is not None
                and (not isinstance(state.request, dict) or reply.request_id != state.request.get('id'))
            ):
                raise ValueError('reply must address the suspended request')
        scratch = REPO / 'playground/stepping'
        scratch.mkdir(parents=True, exist_ok=True)
        with TemporaryDirectory(dir=scratch) as temporary:
            directory = Path(temporary)
            output = directory / 'next.bin'
            if state is None:
                source = directory / 'guest.py'
                source.write_text(
                    (ROOT / 'contracts.py').read_text()
                    + '\n'
                    + (ROOT / 'guest.py').read_text()
                    + '\njson.dumps(entry()._native1())\n'
                )
                args = ['start', str(source), str(output)]
            else:
                snapshot = directory / 'state.bin'
                snapshot.write_bytes(state.snapshot)
                answer = (
                    None
                    if reply is None
                    else ({'raise': reply.error} if reply.error is not None else {'value': reply.value})
                )
                payload = {'native_results': [] if reply is None else [{'reply': answer}]}
                if cancel is not None:
                    payload['native_cancel'] = cancel
                args = ['resume', str(snapshot), json.dumps(payload), str(output)]
            result = subprocess.run(self.command + args, capture_output=True, text=True, timeout=40)
            if result.returncode or result.stderr:
                raise RuntimeError(f'transport failed: {result.stderr or result.stdout}')
            view = json.loads(result.stdout)
            # Receipt-only native IDs do not change the retained-wait semantic view.
            view.pop('raw_pending_ids', None)
            if 'complete' in view:
                return Finished(view['complete'])
            retained = view == {'waiting': True}
            if retained and (state is None or reply is not None or cancel is None):
                raise RuntimeError('retained wait requires a cancellation-only transition')
            return Suspended(output.read_bytes(), state.request if retained else view['effect'], self.backend, retained)

    def cancel(self, state: Suspended, reason: str) -> Suspended | Finished:
        """Request native sticky cancellation; cleanup may return the same outstanding request."""
        return self.advance(state, cancel=reason)
