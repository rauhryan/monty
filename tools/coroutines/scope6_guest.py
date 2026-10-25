"""POLICY PROPOSAL: sequential scopes using only functions, data and generators.

No host clock, imports, decorators or custom classes belong in this guest source.
ValueError is a demonstration carrier, not a proposed public cancellation type.
"""


def request(scope, kind, payload):
    """Attach the complete lexical path to every externally visible boundary."""
    return {'kind': kind, 'scopes': scope['path'], 'phase': scope['phase'], 'payload': payload}


def scoped(scope, body):
    """Preserve pending failure while draining lazy cleanup descriptors in LIFO order."""
    pending = None
    failures = []
    result = None
    yield request(scope, 'enter', None)
    try:
        result = yield from body
    except Exception as error:
        pending = error
    scope['phase'] = 'cleanup'
    while scope['deferred']:
        operation = scope['deferred'].pop()
        try:
            yield request(scope, 'operation', operation)
        except Exception as error:
            failures.append(str(error))
    yield request(scope, 'exit', {'pending': None if pending is None else str(pending), 'failures': failures})
    if failures:
        raise ValueError('cleanup:' + str(failures) + '; prior:' + str(pending))
    if pending is not None:
        raise pending
    return result


def reserve_body(scope):
    """Register release as inert data immediately after successful acquisition."""
    hold = yield request(scope, 'operation', ['reserve', 'seat'])
    scope['deferred'].append(['release', hold])
    return (yield request(scope, 'operation', ['use', hold]))


def nested_body(outer, inner):
    """Keep outer ownership live until inner cleanup and exit have finished."""
    outer['deferred'].append(['release', 'outer-hold'])
    return (yield from scoped(inner, reserve_body(inner)))


def program(nested, deadlines):
    """Use stable host-supplied IDs and opaque absolute deadline data."""
    outer_meta = {'id': 'outer', 'parent': None, 'deadline': deadlines[0]}
    outer = {'path': [outer_meta], 'phase': 'body', 'deferred': []}
    if nested:
        inner_meta = {'id': 'inner', 'parent': 'outer', 'deadline': deadlines[1]}
        inner = {'path': [outer_meta, inner_meta], 'phase': 'body', 'deferred': []}
        return (yield from scoped(outer, nested_body(outer, inner)))
    return (yield from scoped(outer, reserve_body(outer)))
