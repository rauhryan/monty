"""Bounded sequential host kernel: queueing never advances a continuation."""
# ruff: noqa: F821 -- assembled with the guest library and continuation adapter.


def new_state(mode):
    events = []
    failures = []
    return {
        'events': events,
        'failures': failures,
        'generator': adapt(cart(events, failures, mode)),
        'started': False,
        'phase': 'body',
        'status': 'runnable',
        'resume': ['value', None],
        'pending': None,
        'cancel': None,
        'terminal': None,
    }


def observe(state):
    return {
        'events': state['events'],
        'failures': state['failures'],
        'phase': state['phase'],
        'status': state['status'],
        'pending': state['pending'],
        'cancel': state['cancel'],
        'terminal': state['terminal'],
    }


def apply(state, message):
    kind = message['kind']
    if kind == 'cancel':
        if state['cancel'] is None:
            state['cancel'] = message['value']
        if state['phase'] == 'body':
            state['phase'] = 'closing'
            state['pending'] = None
            state['resume'] = ['error', state['cancel']]
            state['status'] = 'runnable'
        return
    if kind == 'settle':
        assert state['status'] == 'waiting'
        state['resume'] = [message['outcome'], message['value']]
        state['pending'] = None
        state['status'] = 'runnable'
        return
    assert kind == 'advance' and state['status'] == 'runnable'
    g = state['generator']
    outcome, value = state['resume']
    try:
        if not state['started']:
            state['started'] = True
            if outcome == 'error':
                raise ValueError(value)
            request = next(g)
        elif outcome == 'error':
            request = g.throw(ValueError(value))
        else:
            request = g.send(value)
    except StopIteration as done:
        state['terminal'] = {'kind': 'value', 'value': done.value, 'cause': None}
    except Exception as error:
        state['terminal'] = {'kind': 'error', 'value': None, 'cause': str(error)}
    else:
        state['pending'] = request
        state['status'] = 'waiting'
        if request[0] == 'release' or request[0] == 'audit-release':
            state['phase'] = 'cleanup'
        return
    state['phase'] = 'done'
    state['status'] = 'done'
    state['generator'] = None
    if state['failures']:
        state['terminal']['kind'] = 'error'
    if state['cancel'] is not None:
        state['terminal']['kind'] = 'cancelled'
        if state['terminal']['cause'] is None:
            state['terminal']['cause'] = state['cancel']
