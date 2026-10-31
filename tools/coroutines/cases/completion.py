"""Completion exception identity, cycles, pending return and repeated catches."""

import json

token = []
events = []


def leaf():
    """Retain the exact return while finally yields."""
    try:
        return token
    finally:
        events.append('pending-return')
        yield token
        events.append('return-finished')


def middle():
    """Delegate completion without copying it."""
    return (yield from leaf())


def outer():
    """Add the second delegate."""
    return (yield from middle())


g = outer()
assert next(g) is token
exchange(json.dumps({'phase': 'pending-return', 'events': events}))
try:
    next(g)
    assert False, 'expected completion'
except StopIteration as done:
    assert done.value is token and done.args[0] is token
    saved = done
    alias = done
token.append(saved)
assert saved.value[0] is saved and alias is saved
exchange(json.dumps({'phase': 'completion-cycle', 'events': events}))
assert saved is alias and saved.value is token and saved.args[0] is token
assert saved.args is saved.args
assert token[0] is saved
try:
    raise saved
except StopIteration as caught:
    assert caught is saved
    try:
        raise
    except StopIteration as again:
        assert again is saved and again.value is token
for operation in [0, 1]:
    try:
        if operation == 0:
            next(g)
        else:
            g.send(None)
        assert False, 'expected exhaustion'
    except StopIteration as empty:
        assert empty.value is None and empty.args == ()


def bad_leaf():
    """Raise an explicit body error rather than return normally."""
    yield None
    raise StopIteration(token)


def bad_middle():
    """Propagate the converted body error."""
    yield from bad_leaf()


def bad_outer():
    """Keep body errors distinct through two delegates."""
    yield from bad_middle()


bad = bad_outer()
assert next(bad) is None
try:
    next(bad)
    assert False, 'expected body StopIteration conversion'
except RuntimeError as error:
    assert str(error) == 'generator raised StopIteration'
json.dumps({'events': events, 'completion_identity': saved.value is token, 'body_stop': 'RuntimeError'})
