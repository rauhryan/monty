"""Independently spelled generator policy reference, without async protocols."""
# ruff: noqa: F821


def resource_exit(state, task, scope, cause):
    scope['cause'] = cause
    scope['phase'] = task['phase'] = 'closing'
    yield {'kind': 'drain', 'scope': scope['id']}
    scope['phase'] = task['phase'] = 'resource-cleanup'
    scope['cursor'] = len(scope['deferred'])
    while scope['cursor']:
        scope['cursor'] -= 1
        try:
            yield {'kind': 'operation', 'scope': scope['id'], 'payload': scope['deferred'][scope['cursor']]}
        except Exception as error:
            scope['failures'].append(str(error))
    close_scope(state, task, scope)


def parent_body(state, task, mode):
    scope = open_scope(state, task, 50)
    scope['deferred'].append(['parent-arbitrary', 'P'])
    try:
        try:
            result = yield from parent_work(state, task, scope, mode)
        except Exception as error:
            yield from resource_exit(state, task, scope, str(error))
            if mode != 'suppress':
                raise
            result = 'after-scope'
        else:
            yield from resource_exit(state, task, scope, None)
        return result
    except ValueError:
        if mode == 'catch':
            return 'caught-parent-success'
        raise


def parent_work(state, task, scope, mode):
    sid = scope['id']
    if mode == 'nested':
        inner = open_scope(state, task, 25)
        inner['deferred'].append(['inner-arbitrary', 'I'])
        try:
            yield {'kind': 'operation', 'scope': inner['id'], 'payload': ['nested-work']}
        except Exception as error:
            yield from resource_exit(state, task, inner, str(error))
            raise
        else:
            yield from resource_exit(state, task, inner, None)
    first = yield {'kind': 'spawn', 'scope': sid, 'label': 'a', 'mode': mode}
    second = yield {'kind': 'spawn', 'scope': sid, 'label': 'b', 'mode': mode}
    if mode == 'early':
        return 'early'
    if mode in ('early-live', 'error', 'suppress', 'break'):
        yield {'kind': 'operation', 'scope': sid, 'payload': ['parent-work']}
        if mode in ('error', 'suppress'):
            raise ValueError('body-original')
        return 'after-scope' if mode == 'break' else 'early-live'
    one = yield {'kind': 'join', 'scope': sid, 'child': first}
    if one['kind'] == 'error' and mode != 'ignore-error':
        raise ValueError('child:failed' if one['cause'] is None else 'child:' + one['cause'])
    two = yield {'kind': 'join', 'scope': sid, 'child': second}
    return [one, two]


def child_body(state, task, mode):
    scope = open_scope(state, task, 40)
    try:
        hold = yield {'kind': 'operation', 'scope': scope['id'], 'payload': ['acquire', task['label']]}
        scope['deferred'].extend([['arbitrary-child', hold], ['another-name', hold]])
        try:
            result = yield {'kind': 'operation', 'scope': scope['id'], 'payload': ['use', hold]}
        except ValueError:
            if mode != 'catch':
                raise
            result = 'caught-child-success'
    except Exception as error:
        yield from resource_exit(state, task, scope, str(error))
        raise
    else:
        yield from resource_exit(state, task, scope, None)
        return result
