"""Persisted cooperative task/scope graph; commands never schedule other tasks."""
# ruff: noqa: F821


def new_driver(mode):
    state = {
        'run': 'R11',
        'mode': mode,
        'next_task': 0,
        'next_scope': 0,
        'next_boundary': 0,
        'tasks': {},
        'scopes': {},
        'events': [],
        'disposed': False,
    }
    add_task(state, None, 'parent', mode)
    return state


def add_task(state, owner, label, mode):
    if owner is not None and not admits_child(state, owner):
        state['events'].append(['reject-spawn', owner])
        return None
    task_id = 'T' + str(state['next_task'])
    state['next_task'] += 1
    task = {
        'id': task_id,
        'owner': owner,
        'label': label,
        'stack': [],
        'scopes': [],
        'phase': 'body',
        'status': 'runnable',
        'started': False,
        'resume': ['value', None],
        'wait': None,
        'joining': None,
        'pending_join': None,
        'draining': None,
        'cancel': None,
        'cancel_pending': False,
        'result': None,
        'consumed': False,
        'failures': [],
        'diagnostics': [],
    }
    state['tasks'][task_id] = task
    if owner is not None:
        state['scopes'][owner]['children'].append(task_id)
    task['generator'] = adapt(parent_body(state, task, mode) if owner is None else child_body(state, task, mode))
    state['events'].append(['task', task_id, owner])
    return task_id


def admits_child(state, scope_id):
    """New contexts cannot reopen cancelled or closing ownership admission."""
    while scope_id is not None:
        scope = state['scopes'][scope_id]
        if scope['phase'] != 'body' or state['tasks'][scope['task']]['cancel'] is not None:
            return False
        scope_id = scope['parent']
    return True


def open_scope(state, task, deadline):
    scope_id = 'S' + str(state['next_scope'])
    state['next_scope'] += 1
    scope = {
        'id': scope_id,
        'task': task['id'],
        'parent': task['stack'][-1] if task['stack'] else task['owner'],
        'phase': 'body',
        'deadline': deadline,
        'children': [],
        'deferred': [],
        'cursor': 0,
        'cause': None,
        'failures': [],
    }
    state['scopes'][scope_id] = scope
    task['stack'].append(scope_id)
    task['scopes'].append(scope_id)
    state['events'].append(['scope', scope_id, task['id'], deadline])
    return scope


def close_scope(state, task, scope):
    assert scope['phase'] == 'resource-cleanup'
    assert all(state['tasks'][child]['status'] == 'done' for child in scope['children'])
    task['failures'].extend(scope['failures'])
    for child_id in scope['children']:
        child = state['tasks'][child_id]
        if (
            (not child['consumed'] and child['result']['kind'] == 'error')
            or child['result']['failures']
            or child['result']['children']
        ):
            task['diagnostics'].append(child['result'])
    assert task['stack'].pop() == scope['id']
    scope['phase'] = 'exited'
    scope['deferred'] = []
    # Admission may be closed while the enclosing continuation is still body code.
    # Restore execution phase independently, including nested protected exits.
    enclosing = state['scopes'][task['stack'][-1]]['phase'] if task['stack'] else 'body'
    task['phase'] = 'body' if enclosing in ('body', 'closing') else enclosing
    state['events'].append(['scope-exit', scope['id'], scope['cause'], scope['failures']])


def cancel_task(state, task_id, reason):
    task = state['tasks'][task_id]
    if task['status'] == 'done':
        return
    first = task['cancel'] is None
    if first:
        task['cancel'] = reason
        task['cancel_pending'] = True
    state['events'].append(['cancel', task_id, reason])
    for scope_id in task['scopes']:
        scope = state['scopes'][scope_id]
        if scope['phase'] == 'body':
            scope['phase'] = 'closing'
        for child in scope['children']:
            cancel_task(state, child, reason)
    if first and task['phase'] == 'body':
        queue_cancel(task)


def queue_cancel(task):
    """Queue the first signal only at a body continuation boundary."""
    task['cancel_pending'] = False
    task['phase'] = 'closing'
    task['wait'] = task['joining'] = task['pending_join'] = None
    task['resume'] = ['error', task['cancel']]
    task['status'] = 'runnable'


def wake_drained(state, task):
    if task['status'] != 'drain':
        return
    scope = state['scopes'][task['draining']]
    if all(state['tasks'][child]['status'] == 'done' for child in scope['children']):
        task['status'] = 'runnable'
        task['resume'] = ['value', None]
        task['draining'] = None
        state['events'].append(['drained', scope['id']])


def finish_task(state, task, kind, value):
    assert not task['stack']
    cause = value if kind == 'error' else task['cancel']
    result_kind = 'error' if cause is not None or task['failures'] or task['diagnostics'] else 'value'
    task['result'] = {
        'kind': 'cancelled' if task['cancel'] is not None else result_kind,
        'value': value if kind == 'value' else None,
        'cause': cause,
        'cancel': task['cancel'],
        'failures': task['failures'],
        'children': task['diagnostics'],
    }
    task['phase'] = 'exited'
    task['cancel_pending'] = False
    task['status'] = 'done'
    task['generator'] = task['wait'] = task['pending_join'] = None
    task['resume'] = ['value', None]
    state['events'].append(['exit', task['id'], task['result']])
    if task['owner'] is not None:
        parent = state['tasks'][state['scopes'][task['owner']]['task']]
        if parent['status'] == 'join' and parent['joining'] == task['id']:
            parent['resume'] = ['value', task['result']]
            parent['pending_join'] = task['id']
            parent['joining'] = None
            parent['status'] = 'runnable'
        wake_drained(state, parent)


def advance(state, task):
    kind, value = task['resume']
    try:
        if not task['started']:
            task['started'] = True
            if kind == 'error':
                finish_task(state, task, 'error', value)
                return
            message = next(task['generator'])
        elif kind == 'error':
            message = task['generator'].throw(ValueError(value))
        else:
            joined = task['pending_join']
            task['pending_join'] = None
            try:
                message = task['generator'].send(value)
            finally:
                if joined is not None:
                    state['tasks'][joined]['consumed'] = True
    except StopIteration as done:
        finish_task(state, task, 'value', done.value)
        return
    except Exception as error:
        finish_task(state, task, 'error', str(error))
        return
    scope = state['scopes'][message['scope']]
    assert scope['task'] == task['id']
    task['resume'] = ['value', None]
    # The send above must finish protected cleanup with its accepted input first.
    # A new body request is a safe throw boundary, before admitting any work.
    if task['cancel_pending'] and task['phase'] == 'body' and message['kind'] != 'drain':
        queue_cancel(task)
        return
    if message['kind'] == 'spawn':
        child_id = add_task(state, scope['id'], message['label'], message['mode'])
        task['resume'] = ['error', 'scope-closed:' + scope['id']] if child_id is None else ['value', child_id]
    elif message['kind'] == 'join':
        if scope['phase'] != 'body':
            task['resume'] = ['error', 'scope-closed:' + scope['id']]
        elif message['child'] not in scope['children']:
            task['resume'] = ['error', 'invalid-join:' + scope['id']]
        else:
            child = state['tasks'][message['child']]
            if child['status'] == 'done':
                task['resume'] = ['value', child['result']]
                task['pending_join'] = child['id']
            else:
                task['status'] = 'join'
                task['joining'] = child['id']
    elif message['kind'] == 'drain':
        assert scope['phase'] == 'closing'
        scope['phase'] = task['phase'] = 'drain'
        task['status'] = 'drain'
        task['draining'] = scope['id']
        state['events'].append(['drain', scope['id'], scope['cause']])
        for child in scope['children']:
            cancel_task(state, child, 'owner-exit:' + scope['id'])
        wake_drained(state, task)
    else:
        boundary = 'B' + str(state['next_boundary'])
        state['next_boundary'] += 1
        task['wait'] = {
            'run': state['run'],
            'task': task['id'],
            'scope': scope['id'],
            'parent_scope': scope['parent'],
            'boundary': boundary,
            'phase': scope['phase'],
            'payload': message['payload'],
        }
        task['status'] = 'wait'
        state['events'].append(['pause', task['wait']])


def command(state, message):
    if state['disposed'] or message['run'] != state['run'] or message['task'] not in state['tasks']:
        state['events'].append(['reject', message])
        return
    task = state['tasks'][message['task']]
    kind = message['kind']
    if kind == 'cancel' and message['scope'] in task['scopes'] and task['status'] != 'done':
        cancel_task(state, task['id'], message['reason'])
    elif kind == 'spawn' and message['scope'] in task['scopes']:
        add_task(state, message['scope'], 'late', 'normal')
    elif kind == 'advance' and task['status'] == 'runnable':
        state['events'].append(['advance', task['id']])
        advance(state, task)
    elif kind == 'settle' and task['wait'] is not None and message['boundary'] == task['wait']['boundary']:
        assert message['outcome'] in ('value', 'error')
        state['events'].append(['accept', task['id'], message['boundary'], message['outcome'], message['value']])
        task['wait'] = None
        task['resume'] = [message['outcome'], message['value']]
        task['status'] = 'runnable'
    else:
        state['events'].append(['reject', message])


def view(state):
    # These aliases/barriers are checked in the live restored graph, before JSON projection.
    for task in state['tasks'].values():
        if task['pending_join'] is not None:
            child = state['tasks'][task['pending_join']]
            assert task['resume'][1] is child['result']
            assert not child['consumed']
        if task['wait'] is not None:
            assert any(event[0] == 'pause' and event[1] is task['wait'] for event in state['events'])
        if task['result'] is not None:
            assert task['result']['failures'] is task['failures']
    for scope in state['scopes'].values():
        if scope['phase'] in ('resource-cleanup', 'exited'):
            assert all(state['tasks'][child]['status'] == 'done' for child in scope['children'])
    return {
        'run': state['run'],
        'pending': [t['wait'] for t in state['tasks'].values() if t['wait'] is not None],
        'runnable': [t['id'] for t in state['tasks'].values() if t['status'] == 'runnable'],
        'events': state['events'],
        'terminal': state['tasks']['T0']['result'],
        'disposed': state['disposed'],
    }
