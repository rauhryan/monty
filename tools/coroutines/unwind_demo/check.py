"""Exercise the demo's actual Wasm snapshot boundary, without a server or builds."""

import tempfile

from server import Demo

with tempfile.TemporaryDirectory(prefix='monty-demo-check-') as directory:
    demo = Demo(directory)

    def act(action):
        return demo.command(action, demo.revision)

    for outcome in ('complete', 'cancel', 'fail'):
        assert act('start')['state']['pending'] == ['charge', {'hold': 'cart-hold-1'}]
        before = demo.snapshot.read_bytes()
        assert act('restore')['snapshot']['bytes'] == len(before)
        assert demo.snapshot.read_bytes() == before
        assert act(outcome)['state']['pending'][0] == 'audit-release'
        if outcome == 'cancel':
            assert act('cancel')['state']['pending'][0] == 'audit-release'
            assert act('fail')['state']['pending'][0] == 'release'
        else:
            assert act('complete')['state']['pending'][0] == 'release'
        final = act('complete')
        assert (
            final['state']['terminal']['kind'] == {'complete': 'value', 'cancel': 'cancelled', 'fail': 'error'}[outcome]
        )
        assert final['state']['failures'] == (['cleanup failed'] if outcome == 'cancel' else [])
        assert final['snapshot']['bytes'] == 0
        print(outcome, 'PASS', flush=True)
    act('start')
    before = demo.state
    assert act('dispose')['disposed']
    assert demo.state == before
    assert not demo.snapshot.exists()
    try:
        demo.command('complete', demo.revision - 1)
    except ValueError:
        pass
    else:
        raise AssertionError('stale reply accepted')
    print('dispose and stale reply PASS')

    act('start')
    act('complete')
    before = demo.snapshot.read_bytes()
    act('restore')
    assert demo.snapshot.read_bytes() == before
    assert act('cancel')['state']['pending'][0] == 'audit-release'
    assert act('complete')['state']['pending'][0] == 'release'
    act('restore')
    final = act('complete')
    assert final['state']['terminal']['kind'] == 'cancelled'
    assert final['state']['terminal']['runtime']['reason'] == 'Cancelled by host'
    assert final['state']['events'][-1] == ['caught', 'Cancelled by host']
    print('deferred cancellation and cleanup restore PASS')
