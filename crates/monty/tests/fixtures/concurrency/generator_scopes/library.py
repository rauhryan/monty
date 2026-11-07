"""Reference semantics: cooperative owned tasks; every step is host selected.

Ordinary data and synchronous generators only. Not a security boundary or a
CPython snapshot implementation. ValueError is a catchable signal carrier.
"""


def new_driver(mode):
    """Root all continuation owners and monotone allocators in one guest graph."""
    state = {
        'mode': mode,
        'run': 'R7',
        'next_task': 0,
        'next_boundary': 0,
        'tasks': {},
        'events': [],
        'disposed': False,
    }
    add_task(state, None, 'parent', mode)
    return state


def add_task(state, parent, label, mode):
    """Admit a child only while its owning scope is open; allocate IDs once."""
    if parent is not None and state['tasks'][parent]['phase'] != 'body':
        state['events'].append(['reject-spawn', parent])
        return None
    number = state['next_task']
    state['next_task'] += 1
    task_id = 'T' + str(number)
    scope_id = 'S' + str(number)
    task = {
        'id': task_id,
        'scope': scope_id,
        'parent': parent,
        'label': label,
        'children': [],
        'parent_scope': None if parent is None else state['tasks'][parent]['scope'],
        'phase': 'body',
        'status': 'runnable',
        'started': False,
        'resume': ['value', None],
        'wait': None,
        'joining': None,
        'pending_join': None,
        'deferred': [],
        'cursor': 0,
        'failures': [],
        'cause': None,
        'cancel': None,
        'value': None,
        'result': None,
        'consumed': False,
    }
    state['tasks'][task_id] = task
    if parent is not None:
        state['tasks'][parent]['children'].append(task_id)
    task['generator'] = parent_body(task, mode) if parent is None else child_body(task, mode)
    state['events'].append(['enter', task_id, scope_id, parent])
    return task_id


def parent_body(task, mode):
    """Catchable signal cannot clear the driver's sticky cancellation."""
    try:
        return (yield from parent_work(task, mode))
    except ValueError:
        if mode == 'catch':
            return 'caught-parent-success'
        raise


def parent_work(task, mode):
    """Explicitly consume child outcomes under the selected non-fail-fast policy."""
    task['deferred'].append(['release', 'parent-H'])
    first = yield {'kind': 'spawn', 'label': 'a', 'mode': mode}
    second = yield {'kind': 'spawn', 'label': 'b', 'mode': mode}
    if mode == 'early':
        return 'early'
    if mode == 'early-live':
        yield {'kind': 'operation', 'payload': ['parent-work']}
        return 'early-live'
    one = yield {'kind': 'join', 'child': first}
    if one['kind'] == 'error' and mode != 'ignore-error':
        # Cleanup-only errors retain their structured outcome in child diagnostics.
        cause = one['cause']
        raise ValueError('child:failed' if cause is None else 'child:' + cause)
    two = yield {'kind': 'join', 'child': second}
    return [one, two]


def child_body(task, mode):
    """Register lazy descriptors only after acquire; catch cancellation explicitly."""
    hold = yield {'kind': 'operation', 'payload': ['acquire', task['label']]}
    task['deferred'].append(['release', hold])
    if mode == 'multi':
        task['deferred'].append(['audit-release', hold])
    try:
        return (yield {'kind': 'operation', 'payload': ['use', hold]})
    except ValueError:
        if mode == 'catch':
            return 'caught-success'
        raise


def cancel_task(state, task_id, reason):
    """Latch cancellation and fan out, without consuming a cleanup boundary."""
    task = state['tasks'][task_id]
    if task['status'] == 'done':
        return
    if task['cancel'] is None:
        task['cancel'] = reason
    state['events'].append(['cancel', task_id, reason])
    for child in task['children']:
        cancel_task(state, child, reason)
    if task['phase'] == 'body':
        task['phase'] = 'closing'
        task['wait'] = None
        task['joining'] = None
        task['pending_join'] = None
        task['resume'] = ['cancel', reason]
        task['status'] = 'runnable'
    # Closing is set before delivery; repeats must not replace the first throw.


def close_body(state, task, kind, value):
    """Close admission, cancel unfinished children, then block on their drain."""
    task['cause'] = value if kind == 'error' else task['cancel']
    task['value'] = value if kind == 'value' else None
    task['phase'] = 'drain'
    task['status'] = 'drain'
    task['wait'] = None
    state['events'].append(['closing', task['id'], kind, value])
    for child_id in task['children']:
        if state['tasks'][child_id]['status'] != 'done':
            cancel_task(state, child_id, 'owner-exit:' + task['id'])
    wake_drained(state, task)


def wake_drained(state, task):
    """Make parent cleanup runnable only after every owned child is terminal."""
    if task['phase'] != 'drain':
        return
    for child_id in task['children']:
        if state['tasks'][child_id]['status'] != 'done':
            return
    task['phase'] = 'cleanup'
    task['cursor'] = len(task['deferred'])
    task['status'] = 'runnable'
    state['events'].append(['drained', task['id']])


def boundary(state, task, payload):
    """Never reuse execution-local effect IDs, even after cancellation."""
    number = state['next_boundary']
    state['next_boundary'] += 1
    task['wait'] = {
        'run': state['run'],
        'task': task['id'],
        'scope': task['scope'],
        'parent_scope': task['parent_scope'],
        'boundary': 'B' + str(number),
        'phase': task['phase'],
        'payload': payload,
    }
    task['status'] = 'wait'
    state['events'].append(['pause', task['wait']])


def finish_task(state, task):
    """Publish results after child drain and LIFO cleanup; keep original cause."""
    unconsumed = []
    for child_id in task['children']:
        child = state['tasks'][child_id]
        if (
            (not child['consumed'] and child['result']['kind'] == 'error')
            or child['failures']
            or child['result']['children']
        ):
            unconsumed.append(child['result'])
    kind = 'value'
    if task['cause'] is not None or task['failures'] or unconsumed:
        kind = 'error'
    if task['cancel'] is not None:
        kind = 'cancelled'
    task['result'] = {
        'kind': kind,
        'value': task['value'],
        'cause': task['cause'],
        'cancel': task['cancel'],
        'failures': task['failures'],
        'children': unconsumed,
    }
    task['status'] = 'done'
    task['phase'] = 'exited'
    task['generator'] = None
    task['deferred'] = []
    task['resume'] = ['value', None]
    task['pending_join'] = None
    state['events'].append(['exit', task['id'], task['result']])
    if task['parent'] is not None:
        parent = state['tasks'][task['parent']]
        if parent['status'] == 'join' and parent['joining'] == task['id']:
            parent['resume'] = ['value', task['result']]
            parent['pending_join'] = task['id']
            parent['status'] = 'runnable'
            parent['joining'] = None
        wake_drained(state, parent)


def advance(state, task):
    """Perform one host-authorized synchronous slice, never run other tasks."""
    if task['phase'] == 'cleanup':
        if task['cursor'] > 0:
            task['cursor'] -= 1
            boundary(state, task, task['deferred'][task['cursor']])
        else:
            finish_task(state, task)
        return
    kind, value = task['resume']
    try:
        if not task['started']:
            task['started'] = True
            if kind == 'cancel':
                # Unstarted tasks have no acquired resources or authored prefix.
                close_body(state, task, 'error', value)
                return
            request = next(task['generator'])
        elif kind in ('cancel', 'error'):
            request = task['generator'].throw(ValueError(value))
        else:
            # Queueing is not delivery. Consume only across this actual send,
            # including when the receiving body returns or propagates an error.
            joined = task['pending_join']
            task['pending_join'] = None
            try:
                request = task['generator'].send(value)
            finally:
                if joined is not None:
                    state['tasks'][joined]['consumed'] = True
    except StopIteration as done:
        close_body(state, task, 'value', done.value)
        return
    except Exception as error:
        close_body(state, task, 'error', str(error))
        return
    if request['kind'] == 'spawn':
        child = add_task(state, task['id'], request['label'], request['mode'])
        task['resume'] = ['value', child]
    elif request['kind'] == 'join':
        child = state['tasks'][request['child']]
        assert child['parent'] == task['id']
        if child['status'] == 'done':
            task['resume'] = ['value', child['result']]
            task['pending_join'] = child['id']
        else:
            task['joining'] = child['id']
            task['status'] = 'join'
    else:
        boundary(state, task, request['payload'])


def command(state, message):
    """Validate routing before mutation; settlements enqueue, advances execute."""
    if state['disposed'] or message['run'] != state['run']:
        state['events'].append(['reject', message])
        return
    kind = message['kind']
    if kind == 'dispose':
        # CPython must retain these generators separately: reference loss can close.
        state['disposed'] = True
        state['events'].append(['disposed-compute'])
        return
    task_id = message['task']
    if task_id not in state['tasks']:
        state['events'].append(['reject', message])
        return
    task = state['tasks'][task_id]
    if kind == 'cancel':
        if message['scope'] != task['scope'] or task['status'] == 'done':
            state['events'].append(['reject', message])
        else:
            cancel_task(state, task_id, message['reason'])
    elif kind == 'spawn':
        add_task(state, task_id, 'late', 'normal')
    elif kind == 'advance' and task['status'] == 'runnable':
        state['events'].append(['advance', task_id])
        advance(state, task)
    elif kind == 'settle' and task['wait'] is not None and message['boundary'] == task['wait']['boundary']:
        assert message['outcome'] in ('value', 'error')
        state['events'].append(['accept', task_id, message['boundary'], message['outcome'], message['value']])
        task['wait'] = None
        task['status'] = 'runnable'
        if task['phase'] == 'cleanup':
            if message['outcome'] == 'error':
                task['failures'].append(message['value'])
        else:
            task['resume'] = [message['outcome'], message['value']]
    else:
        state['events'].append(['reject', message])


def view(state):
    """Expose JSON observations, never export generators or guest error objects."""
    pending = []
    runnable = []
    for task_id in state['tasks']:
        task = state['tasks'][task_id]
        if task['wait'] is not None:
            pending.append(task['wait'])
        if task['status'] == 'runnable':
            runnable.append(task_id)
    return {
        'run': state['run'],
        'pending': pending,
        'runnable': runnable,
        'events': state['events'],
        'terminal': state['tasks']['T0']['result'],
        'disposed': state['disposed'],
    }
