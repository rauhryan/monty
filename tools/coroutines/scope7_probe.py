"""Assemble the same guest library for a next native/actual-Wasm exchange probe.

--write PATH writes runnable guest source. Default checks the assembled source
with CPython and exact host scripts, not Monty, Wasm or serialization.
"""

import json
import sys
from pathlib import Path

import scope7_reference
from scope7_reference import act, exact_trace, scenarios

BRIDGE = """
import json

config = json.loads(exchange(json.dumps({'kind': 'configure', 'contract': 7})))
state = new_driver(config['mode'])
while state['tasks']['T0']['result'] is None and not state['disposed']:
    message = json.loads(exchange(json.dumps(view(state))))
    command(state, message)
json.dumps(view(state))
"""


def source():
    """Use only the existing external exchange(JSON text) adapter contract."""
    return Path(__file__).with_name('scope7_guest.py').read_text() + BRIDGE


def check_bridge(mode, messages):
    """Replay recorded host commands through the actual assembled JSON bridge."""
    position = 0
    observed = []

    def exchange(payload):
        """Supply configuration then exactly one previously selected host command."""
        nonlocal position
        item = json.loads(payload)
        observed.append(item)
        if item.get('kind') == 'configure':
            return json.dumps({'mode': mode})
        assert position < len(messages)
        message = messages[position]
        position += 1
        return json.dumps(message)

    namespace = {'exchange': exchange}
    exec(compile(source(), 'scope7-guest-probe.py', 'exec'), namespace)
    assert position == len(messages)
    return namespace['state'], observed


def main():
    """Emit source or compare CPython direct and JSON-bridge observations."""
    if len(sys.argv) == 3 and sys.argv[1] == '--write':
        Path(sys.argv[2]).write_text(source())
        print('Guest probe written; native/Wasm execution NOT tested.')
        return
    # Record commands by intercepting the reference's single command gateway.
    # This affects only the CPython host module, never the guest source.
    recordings = []
    current = None

    def record(state, kind, task='T0', **fields):
        """Associate each newly created aggregate heap with its host script."""
        nonlocal current
        if current is not state:
            recordings.append([])
            current = state
        recordings[-1].append({'run': 'R7', 'kind': kind, 'task': task, **fields})
        return act(state, kind, task, **fields)

    scope7_reference.act = record
    states = scenarios()
    assert len(recordings) == len(states)
    for (name, direct), messages in zip(states.items(), recordings, strict=True):
        # Physical disposal belongs to the host at a suspended exchange. The
        # bridge ends on logical dispose; a subsequent reply is host-rejected.
        if name == 'compute_only_dispose':
            messages = messages[:-1]
        replayed, observations = check_bridge(direct['mode'], messages)
        trace = exact_trace(direct)
        if name == 'compute_only_dispose':
            trace = trace[:-1]
        assert exact_trace(replayed) == trace
        print(f'{name}: JSON bridge PASS ({len(observations)} exchanges; CPython only)')


if __name__ == '__main__':
    main()
