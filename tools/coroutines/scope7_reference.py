"""CPython-only owned-task contract; exact traces are separate from kernel cases."""

import json
import platform
import sys
from pathlib import Path

from scope7_guest import command, new_driver, view

# CPython reference loss can execute finally. Retain disposed heaps until process
# exit; logical dispose below never calls close/throw/send or claims native free.
DISPOSED = []


def act(state, kind, task='T0', **fields):
    """One host command; roundtrip observations detach mutable guest aliases."""
    message = {'run': 'R7', 'kind': kind, 'task': task, **fields}
    command(state, message)
    observed = json.loads(json.dumps(view(state)))
    root = state['tasks']['T0']
    children = [state['tasks'][child] for child in root['children']]
    if any(child['status'] != 'done' for child in children):
        assert root['result'] is None
        assert root['phase'] != 'cleanup'
    return observed


def step(state, task='T0'):
    """Host explicitly picks a runnable continuation, with no queue auto-drain."""
    return act(state, 'advance', task)


def settle(state, task, outcome='value', value=None):
    """Accept one current boundary without advancing any continuation."""
    wait = state['tasks'][task]['wait']
    return act(state, 'settle', task, boundary=wait['boundary'], outcome=outcome, value=value)


def cancel(state, task='T0', reason='alarm'):
    """Scope intent is distinct from a stale boundary-addressed I/O reply."""
    return act(state, 'cancel', task, scope=state['tasks'][task]['scope'], reason=reason)


def setup(mode='normal', *, join=True):
    """Park both children at I/O; optionally delay the parent's join of a."""
    state = new_driver(mode)
    step(state)  # Spawn a; child prefix has not run.
    step(state)  # Spawn b.
    if join:
        step(state)  # Join a (or early return/close).
    if mode != 'early':
        for child, hold in [('T1', 'A-H'), ('T2', 'B-H')]:
            step(state, child)
            settle(state, child, value=hold)
            step(state, child)
    return state


def finish_child(state, task, value):
    """Host settles use, explicitly starts cleanup, settles release, then exits."""
    settle(state, task, value=value)
    step(state, task)
    step(state, task)
    settle(state, task)
    step(state, task)


def drain_cancelled_child(state, task):
    """Cancellation injection and yielding cleanup remain separate host turns."""
    step(state, task)
    step(state, task)
    settle(state, task)
    step(state, task)


def finish_parent(state):
    """After all children exit, drive the parent's deferred release to completion."""
    step(state)
    settle(state, 'T0')
    step(state)
    assert state['tasks']['T0']['status'] == 'done'


def scenarios():
    """Exercise the required policies with focused, ordered host scripts."""
    results = {}
    for order in [('T1', 'T2'), ('T2', 'T1')]:
        state = setup()
        for child in order:
            finish_child(state, child, child + '-value')
        step(state)  # Join b, already done.
        step(state)  # Parent returns; children drained.
        finish_parent(state)
        results['normal_' + '_'.join(order)] = state

    state = setup()
    stale = dict(state['tasks']['T1']['wait'])
    cancel(state)
    step(state)  # Parent acknowledges cancel; cannot enter resource cleanup.
    act(state, 'spawn')  # Closing scope rejects admission without allocating.
    step(state, 'T1')
    step(state, 'T1')  # Release is now pending.
    pending = json.loads(json.dumps(state['tasks']['T1']['wait']))
    cancel(state, reason='again')
    assert state['tasks']['T1']['wait'] == pending
    act(state, 'settle', 'T1', boundary=stale['boundary'], outcome='value', value='late-use')
    settle(state, 'T1')
    step(state, 'T1')
    drain_cancelled_child(state, 'T2')
    finish_parent(state)
    results['parent_alarm_drain_repeat_stale_spawn'] = state

    state = setup('catch')
    cancel(state)
    step(state)
    for child in ['T2', 'T1']:
        drain_cancelled_child(state, child)
        assert state['tasks'][child]['result']['value'] == 'caught-success'
        assert state['tasks'][child]['result']['kind'] == 'cancelled'
    finish_parent(state)
    assert state['tasks']['T0']['result']['value'] == 'caught-parent-success'
    assert state['tasks']['T0']['result']['kind'] == 'cancelled'
    results['caught_success_sticky_cancel'] = state

    state = setup()
    settle(state, 'T1', outcome='error', value='use-failed')
    step(state, 'T1')
    step(state, 'T1')
    settle(state, 'T1')
    step(state, 'T1')
    # Failure has not independently cancelled sibling or parent: explicit join
    # consumption wakes the parent, whose author chooses to propagate the error.
    assert state['tasks']['T2']['status'] == 'wait'
    assert state['tasks']['T0']['cancel'] is None
    step(state)  # Raise child:use-failed, close parent, cancel sibling.
    drain_cancelled_child(state, 'T2')
    finish_parent(state)
    results['child_failure_sibling_pending'] = state

    state = setup('multi')
    settle(state, 'T1', outcome='error', value='original-use-error')
    step(state, 'T1')
    step(state, 'T1')
    settle(state, 'T1', outcome='error', value='audit-failed')
    step(state, 'T1')
    settle(state, 'T1', outcome='error', value='release-failed')
    step(state, 'T1')
    step(state)  # Parent consumes error; sibling is cancelled.
    step(state, 'T2')
    step(state, 'T2')
    settle(state, 'T2', outcome='error', value='sibling-audit-failed')
    step(state, 'T2')
    settle(state, 'T2')
    step(state, 'T2')
    finish_parent(state)
    results['multiple_cleanup_errors_original_cause'] = state

    state = setup('early')
    # Neither child has started: owner exit closes admission and drains without
    # executing either child's acquire prefix.
    act(state, 'spawn')
    for child in ['T2', 'T1']:
        step(state, child)
        step(state, child)
    finish_parent(state)
    results['normal_exit_cancels_unstarted_children'] = state

    state = setup('early-live')
    settle(state, 'T0', value='parent-done')
    step(state)
    for child in ['T1', 'T2']:
        drain_cancelled_child(state, child)
    finish_parent(state)
    assert state['tasks']['T0']['result']['kind'] == 'value'
    results['normal_exit_cancels_live_children'] = state

    state = setup('early-live')
    settle(state, 'T1', outcome='error', value='unconsumed-use-error')
    step(state, 'T1')
    step(state, 'T1')
    settle(state, 'T1')
    step(state, 'T1')
    assert state['tasks']['T0']['status'] == 'wait'
    assert state['tasks']['T2']['status'] == 'wait'
    settle(state, 'T0', value='parent-done')
    step(state)
    drain_cancelled_child(state, 'T2')
    finish_parent(state)
    assert state['tasks']['T0']['result']['kind'] == 'error'
    assert state['tasks']['T0']['result']['children'][0]['cause'] == 'unconsumed-use-error'
    results['unconsumed_child_error_at_exit'] = state

    for already_done in [False, True]:
        path = 'already_done' if already_done else 'wakeup'
        state = setup(join=not already_done)
        settle(state, 'T1', outcome='error', value='R7-UNDELIVERED-JOIN-ERROR')
        step(state, 'T1')
        step(state, 'T1')
        settle(state, 'T1')
        step(state, 'T1')
        if already_done:
            step(state)  # Join the completed child; still only queue its result.
        assert state['tasks']['T0']['pending_join'] == 'T1'
        assert state['tasks']['T0']['resume'][1] is state['tasks']['T1']['result']
        assert not state['tasks']['T1']['consumed']
        cancel(state, reason='R7-CANCEL-BEFORE-JOIN-DELIVERY')
        assert state['tasks']['T0']['pending_join'] is None
        step(state)
        step(state, 'T2')
        step(state, 'T2')
        pending = dict(state['tasks']['T2']['wait'])
        cancel(state, reason='R8-REPEAT-DURING-CLEANUP')
        assert state['tasks']['T2']['wait'] == pending
        settle(state, 'T2')
        step(state, 'T2')
        finish_parent(state)
        # Independently specified diagnostic, not copied from observed output.
        assert state['tasks']['T0']['result'] == {
            'kind': 'cancelled',
            'value': None,
            'cause': 'R7-CANCEL-BEFORE-JOIN-DELIVERY',
            'cancel': 'R7-CANCEL-BEFORE-JOIN-DELIVERY',
            'failures': [],
            'children': [
                {
                    'kind': 'error',
                    'value': None,
                    'cause': 'R7-UNDELIVERED-JOIN-ERROR',
                    'cancel': None,
                    'failures': [],
                    'children': [],
                }
            ],
        }
        assert not state['tasks']['T1']['consumed']
        results['cancel_before_join_delivery_' + path] = state

        state = setup('ignore-error', join=not already_done)
        settle(state, 'T1', outcome='error', value='R8-DELIVERED-IGNORED-ERROR')
        step(state, 'T1')
        step(state, 'T1')
        settle(state, 'T1')
        step(state, 'T1')
        if already_done:
            step(state)
        assert not state['tasks']['T1']['consumed']
        step(state)  # Actually deliver; this body deliberately ignores the error.
        assert state['tasks']['T1']['consumed']
        assert state['tasks']['T0']['pending_join'] is None
        finish_child(state, 'T2', 'R8-SIBLING-OK')
        step(state)
        finish_parent(state)
        root = state['tasks']['T0']['result']
        assert root['kind'] == 'value'
        assert root['cause'] is None and root['failures'] == [] and root['children'] == []
        assert root['value'][0]['cause'] == 'R8-DELIVERED-IGNORED-ERROR'
        results['delivered_join_error_ignored_' + path] = state

    state = setup()
    settle(state, 'T1', value='R7-CHILD-BODY-OK')
    step(state, 'T1')
    step(state, 'T1')
    settle(state, 'T1', outcome='error', value='R7-CLEANUP-ONLY-ERROR')
    step(state, 'T1')
    assert not state['tasks']['T1']['consumed']
    step(state)
    assert state['tasks']['T1']['consumed']
    drain_cancelled_child(state, 'T2')
    finish_parent(state)
    assert state['tasks']['T0']['result'] == {
        'kind': 'error',
        'value': None,
        'cause': 'child:failed',
        'cancel': None,
        'failures': [],
        'children': [
            {
                'kind': 'error',
                'value': 'R7-CHILD-BODY-OK',
                'cause': None,
                'cancel': None,
                'failures': ['R7-CLEANUP-ONLY-ERROR'],
                'children': [],
            }
        ],
    }
    results['cleanup_only_child_failure'] = state

    state = setup('multi')
    cancel(state)
    step(state)
    for child in ['T2', 'T1']:
        step(state, child)
        step(state, child)
        settle(state, child, outcome='error', value=child + '-audit-failed')
        step(state, child)
        settle(state, child, outcome='error', value=child + '-release-failed')
        step(state, child)
    finish_parent(state)
    assert state['tasks']['T0']['result']['kind'] == 'cancelled'
    results['cancel_cleanup_failures_original_cause'] = state

    state = setup()
    results['compute_only_dispose'] = state
    before = len(state['events'])
    act(state, 'dispose')
    act(state, 'settle', 'T1', boundary='B1', outcome='value', value='late')
    assert state['events'][before][0] == 'disposed-compute'
    assert len(state['events']) == before + 2
    DISPOSED.append(state)
    return results


def exact_trace(state):
    """Full event payloads, including IDs/results/failures, compared exactly."""
    return [json.dumps(event, sort_keys=True, separators=(',', ':')) for event in state['events']]


def main():
    """Fail on any exact-trace difference; this is never the accepted VM oracle."""
    assert platform.python_implementation() == 'CPython'
    states = scenarios()
    expected = json.loads(Path(__file__).with_name('scope7_expected.json').read_text())
    assert set(states) == set(expected)
    for name, state in states.items():
        trace = exact_trace(state)
        assert trace == expected[name], (name, trace, expected[name])
        print(f'{name}: PASS ({len(trace)} exact events)')
        if '--json' in sys.argv:
            print(json.dumps({'scenario': name, **view(state)}, sort_keys=True))
    print('CPython reference semantics only; no Monty/snapshot execution claimed.')


if __name__ == '__main__':
    main()
