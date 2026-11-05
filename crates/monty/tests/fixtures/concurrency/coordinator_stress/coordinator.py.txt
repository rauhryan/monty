"""FIFO cooperative coordinator, executed inside the durable guest snapshot."""
# ruff: noqa: F821


def guest_cancel(state, task, reason):
    # A one-shot catchable signal, unlike the sticky execution cancellation.
    if task['status'] == 'done' or task['phase'] != 'body':
        return
    task['guest_signal'] = 'task-cancel:' + reason
    task['wait'] = task['joining'] = task['pending_join'] = task['io'] = None
    task['resume'] = ['error', task['guest_signal']]
    task['status'] = 'runnable'
    state['events'].append(['task-cancel', task['id'], reason])


def deliver_io(state, task):
    message = task['io']
    queue = state['queues'][message['queue']]
    kind = message['kind']
    if queue[kind][0] != task['id'] or (not queue['items'] if kind == 'get' else len(queue['items']) == 1):
        task['status'] = 'queue-' + kind
        return False
    queue[kind].pop(0)
    value = queue['items'].pop(0) if kind == 'get' else None
    if kind == 'put':
        queue['items'].append(message['value'])
    task['io'] = None
    task['resume'] = ['value', value]
    state['events'].append([kind, task['id'], message['value'] if kind == 'put' else value])
    return True


def reconcile(state, current=None):
    # Reservations stay in the queue until delivery. Cancellation can revoke a
    # wake without losing an item or committing an undelivered put.
    for queue in state['queues'].values():
        for kind in ['get', 'put']:
            queue[kind] = [t for t in queue[kind] if state['tasks'][t]['io'] is not None]
            possible = bool(queue['items']) if kind == 'get' else len(queue['items']) < 1
            if queue[kind] and possible:
                state['tasks'][queue[kind][0]]['status'] = 'runnable'
    state['ready'] = [t for t in state['ready'] if state['tasks'][t]['status'] == 'runnable']
    for task in state['tasks'].values():
        if task['id'] != current and task['status'] == 'runnable' and task['id'] not in state['ready']:
            state['ready'].append(task['id'])
    if current is not None and state['tasks'][current]['status'] == 'runnable' and current not in state['ready']:
        state['ready'].append(current)


def turn(state, messages, budget):
    # All ordered inputs are accepted before any continuation runs.
    for message in messages:
        duplicate = message['event'] in state['seen']
        if not state['retain_history']:
            duplicate = not isinstance(message['event'], int) or message['event'] <= state['event_watermark']
        if message['run'] != state['run'] or duplicate:
            state['events'].append(['reject', message])
            continue
        if state['retain_history']:
            state['seen'].append(message['event'])
        else:
            state['event_watermark'] = message['event']
        if message['kind'] == 'cancel':
            assert isinstance(message['reason'], str), 'cancel reason must be a string'
            if not state['host_cancelled']:
                state['host_cancelled'] = True
                state['host_reason'] = message['reason']
            cancel_task(state, 'T0', message['reason'])
        elif message['kind'] == 'reply':
            if message['outcome'] not in ('value', 'error'):
                state['events'].append(['reject', message])
                continue
            found = False
            for task in state['tasks'].values():
                if task['wait'] is not None and task['wait']['boundary'] == message['operation']:
                    task['wait'] = None
                    task['resume'] = [message['outcome'], message['value']]
                    task['status'] = 'runnable'
                    state['events'].append(['accept', message['operation'], message['outcome'], message['value']])
                    found = True
                    break
            if not found:
                state['events'].append(['reject', message])
        else:
            state['events'].append(['reject', message])
        reconcile(state)
    reconcile(state)
    for _ in range(budget):
        if not state['ready']:
            break
        task_id = state['ready'].pop(0)
        task = state['tasks'][task_id]
        state['events'].append(['advance', task_id])
        advance(state, task)
        if task['io'] is not None:
            queue = state['queues'][task['io']['queue']]
            waiters = queue[task['io']['kind']]
            if task_id not in waiters:
                waiters.append(task_id)
        reconcile(state, task_id)
        reclaim(state)
    reclaim(state)
    if not state['ready'] and not any(t['wait'] is not None for t in state['tasks'].values()):
        assert state['tasks']['T0']['status'] == 'done', 'internal-deadlock'


def observe(state):
    result = view(state)
    result['ready'] = state['ready']
    result['queues'] = state['queues']
    result['tasks'] = [
        [t['id'], t['status'], t['phase'], t['joining'], t['pending_join'], t['consumed'], t['result']]
        for t in state['tasks'].values()
    ]
    result['scopes'] = [[s['id'], s['parent'], s['phase'], s['children']] for s in state['scopes'].values()]
    result['host_cancelled'] = state['host_cancelled']
    result['host_reason'] = state['host_reason']
    result['boundary'] = 'complete' if result['terminal'] is not None else ('runnable' if state['ready'] else 'blocked')
    return result


def reclaim(state):
    """Release delivered outcomes; transfer required diagnostics before unlinking."""
    if state['retain_history']:
        return
    for task_id in list(state['tasks']):
        task = state['tasks'][task_id]
        if task_id == 'T0' or task['status'] != 'done':
            continue
        owner = state['scopes'][task['owner']]
        if not task['consumed'] and owner['phase'] != 'exited':
            continue
        # close_scope already transferred all required diagnostics at scope exit.
        if owner['phase'] != 'exited' and (task['result']['failures'] or task['result']['children']):
            state['tasks'][owner['task']]['diagnostics'].append(task['result'])
        owner['children'].remove(task_id)
        state['tasks'].pop(task_id)
        state['reclaimed_tasks'] += 1
    for scope_id in list(state['scopes']):
        scope = state['scopes'][scope_id]
        if scope['phase'] == 'exited' and not scope['children']:
            if scope['task'] in state['tasks']:
                state['tasks'][scope['task']]['scopes'].remove(scope_id)
            state['scopes'].pop(scope_id)
    # Event history is diagnostic, not an operation/outcome ownership root.
    # Pending waits and replies remain in task records independently of this tail.
    state['events'] = state['events'][-64:]
