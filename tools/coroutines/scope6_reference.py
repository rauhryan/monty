"""Executable POLICY PROPOSAL, separate from accepted kernel fixtures.

Run with CPython from any directory. Exact compact traces are checked; --json
also emits full pause envelopes. This is a scripted host, not a scheduler.
"""

import json
import platform
import sys
from pathlib import Path

from scope6_guest import program


def drive(name, nested, actions):
    """Admit one reply per pause; coalesce cancellation while cleanup is pending."""
    guest = program(nested, [300000, 120000])
    trace = []
    envelopes = []
    pause = None
    turn = 0
    cancelled = False

    def advance(kind, value):
        """A consuming reply steps the generator synchronously to its next boundary."""
        nonlocal pause, turn
        try:
            if kind == 'start':
                item = next(guest)
            elif kind in ('cancel', 'error'):
                item = guest.throw(ValueError(value))
            else:
                item = guest.send(value)
        except StopIteration as done:
            pause = None
            trace.append('terminal cancelled' if cancelled else 'terminal value=' + str(done.value))
            return
        except ValueError as error:
            pause = None
            trace.append('terminal error=' + str(error))
            return
        turn += 1
        # Serialization detaches the host observation from mutable guest data.
        pause = json.loads(json.dumps({'v': 1, 'run': 'R', 'task': 'T', 'turn': turn, **item}))
        envelopes.append(pause)
        path = '/'.join(meta['id'] + '@' + str(meta['deadline']) for meta in pause['scopes'])
        trace.append(
            f'pause {turn} {pause["kind"]} {path} {pause["phase"]} {json.dumps(pause["payload"], separators=(",", ":"))}'
        )
        for index, meta in enumerate(pause['scopes']):
            assert meta['parent'] == (None if index == 0 else pause['scopes'][index - 1]['id'])

    advance('start', None)
    for reply_turn, kind, value in actions:
        if pause is None or reply_turn != pause['turn']:
            trace.append(f'reject {reply_turn} {kind}')
            continue
        if kind == 'cancel' and pause['phase'] == 'cleanup':
            cancelled = True
            trace.append(f'coalesce {reply_turn} cancel')
            continue
        # Enter/exit are control records: host must acknowledge them before any
        # cancellation delivery, to avoid a half-entered or half-exited scope.
        if pause['kind'] != 'operation' and kind != 'value':
            trace.append(f'reject-control {reply_turn} {kind}')
            continue
        if kind == 'cancel':
            cancelled = True
        trace.append(f'accept {reply_turn} {kind}={value}')
        advance(kind, value)
    assert pause is None, (name, 'script left a continuation pending')
    return trace, envelopes


def main():
    """Check hand-authored exact traces for both alarm race winners and nesting."""
    assert platform.python_implementation() == 'CPython'
    scenarios = {
        'normal_reply_wins': (
            False,
            [
                (1, 'value', None),
                (2, 'value', 'H'),
                (2, 'value', 'duplicate-H'),
                (3, 'value', 'used'),
                (3, 'cancel', 'cancel:alarm'),
                (4, 'value', None),
                (5, 'value', None),
                (5, 'value', 'late'),
            ],
        ),
        'alarm_wins': (
            False,
            [
                (1, 'cancel', 'cancel:early'),
                (1, 'value', None),
                (2, 'value', 'H'),
                (3, 'cancel', 'cancel:alarm'),
                (3, 'value', 'late-use'),
                (4, 'cancel', 'cancel:again'),
                (4, 'value', None),
                (5, 'value', None),
            ],
        ),
        'nested_cancel': (
            True,
            [
                (1, 'value', None),
                (2, 'value', None),
                (3, 'value', 'H'),
                (4, 'cancel', 'cancel:alarm'),
                (5, 'value', None),
                (6, 'value', None),
                (7, 'value', None),
                (8, 'value', None),
            ],
        ),
        'cleanup_failure': (
            False,
            [
                (1, 'value', None),
                (2, 'value', 'H'),
                (3, 'cancel', 'cancel:alarm'),
                (4, 'error', 'release-failed'),
                (5, 'value', None),
            ],
        ),
        'cancel_before_acquire': (
            False,
            [
                (1, 'value', None),
                (2, 'cancel', 'cancel:alarm'),
                (2, 'value', 'late-H'),
                (3, 'value', None),
            ],
        ),
    }
    # These are new proposal expectations. Never import or rewrite the accepted oracle.
    expected = json.loads(Path(__file__).with_name('scope6_expected.json').read_text())
    assert set(expected) == set(scenarios)
    for name, (nested, actions) in scenarios.items():
        trace, envelopes = drive(name, nested, actions)
        assert trace == expected[name], (name, trace, expected[name])
        print(f'{name}: PASS ({len(trace)} exact events)')
        if '--json' in sys.argv:
            print(json.dumps({'scenario': name, 'trace': trace, 'pauses': envelopes}, sort_keys=True))
    print('CPython live generators only; no Monty or snapshot execution claimed.')


if __name__ == '__main__':
    main()
