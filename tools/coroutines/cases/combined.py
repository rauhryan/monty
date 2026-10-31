"""Active nested host cut, dormant throw, cleanup cut, and pending-error finish."""

import json

events = []
token = []
token.append(token)
alias = token


def leaf():
    """Cross active, dormant and pending-error cleanup states."""
    events.append('prefix')
    assert token is alias and token[0] is token
    exchange(json.dumps({'phase': 'active', 'events': events}))
    assert token is alias and token[0] is token and events == ['prefix']
    events.append('restored-active')
    try:
        yield token
    finally:
        events.append('cleanup-enter')
        yield token
        assert token is alias and token[0] is token
        events.append('cleanup-finish')


def middle():
    """Keep the first delegate active at the internal host call."""
    yield from leaf()


def outer():
    """Keep the second delegate active at the internal host call."""
    yield from middle()


g = outer()
g_alias = g
assert next(g) is token
assert g_alias is g
reply = json.loads(exchange(json.dumps({'phase': 'dormant', 'events': events})))
assert reply == {'throw': 'injected'}
assert g.throw(ValueError(reply['throw'])) is token
assert events == ['prefix', 'restored-active', 'cleanup-enter']
exchange(json.dumps({'phase': 'cleanup-yield', 'events': events}))
assert g is g_alias and token is alias and token[0] is token
try:
    g.send(None)
    assert False, 'expected pending error'
except ValueError as error:
    assert str(error) == 'injected'
    events.append('caught-injected')
assert events == ['prefix', 'restored-active', 'cleanup-enter', 'cleanup-finish', 'caught-injected']
json.dumps({'events': events, 'identity': token is alias and token[0] is token})
