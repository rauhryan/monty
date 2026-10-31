"""Run real native/Wasm stepping with deterministic host replies and cleanup."""

import argparse
import json

from host import Finished, Reply, Sandbox, Suspended


def main():
    """Show the host only handling operations, never selecting guest tasks."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backend', choices=('native', 'wasm'), default='native')
    parser.add_argument('--fail-charge', action='store_true')
    parser.add_argument('--cancel', action='store_true')
    parser.add_argument('--cancel-in-cleanup', action='store_true')
    args = parser.parse_args()
    sandbox = Sandbox(args.backend)
    state = sandbox.advance()
    names = []
    while isinstance(state, Suspended):
        request = state.request
        names.append(request['name'])
        print(json.dumps({'suspended': request, 'snapshot_bytes': len(state.snapshot)}))
        if args.cancel and request['name'] == 'charge':
            state = Sandbox(args.backend).cancel(state, '')
            continue
        values = {'reserve': {'id': 'hold-1', 'total': 42}, 'charge': 'receipt-1', 'release': None}
        error = 'payment declined' if args.fail_charge and request['name'] == 'charge' else None
        reply = Reply(request['id'], values[request['name']], error)
        # Reconstructing the facade demonstrates that only snapshot + input survive.
        state = Sandbox(args.backend).advance(
            state,
            reply,
            cancel=('REPEAT' if args.cancel else '')
            if request['name'] == 'release' and (args.cancel or args.cancel_in_cleanup)
            else None,
        )
    assert isinstance(state, Finished)
    assert names == ['reserve', 'charge', 'release']
    expected = (
        {'status': 'failed', 'error': 'payment declined'}
        if args.fail_charge
        else {'status': 'succeeded', 'value': 'receipt-1'}
    )
    if args.cancel or args.cancel_in_cleanup:
        assert state.outcome['status'] == 'cancelled' and state.outcome['reason'] == ''
        assert state.outcome['error'] is None
        expected = {'status': 'failed', 'error': ''}
        assert json.loads(state.outcome['value']) == expected
    else:
        assert state.outcome == expected
    print(json.dumps({'finished': state.outcome}))


if __name__ == '__main__':
    main()
