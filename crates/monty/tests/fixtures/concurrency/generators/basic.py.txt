"""Lazy binding, send identity and repeated exhaustion without a payload return."""
import json

events = []
token = []
token.append(token)


def leaf(value):
    events.append('entered')
    reply = yield value
    assert reply is value and value[0] is value
    events.append('sent')
    yield value


g = leaf(token)
alias = g
assert events == []
try:
    g.send(token)
    assert False
except TypeError:
    pass
assert events == []
assert next(g) is token
assert alias.send(token) is token
assert next(g, 'done') == 'done'
assert next(g, 'done-again') == 'done-again'
json.dumps({'events': events, 'identity': alias is g})
