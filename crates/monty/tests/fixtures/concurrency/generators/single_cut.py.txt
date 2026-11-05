"""Active host cut then dormant cut; no replay, aliases, captured cells, disposal."""
import json

events = []
token = []
token.append(token)


def factory(value):
    def leaf():
        events.append('entered')
        exchange(json.dumps({'phase': 'active', 'events': events}))
        assert value is token and value[0] is value
        try:
            received = yield value
            assert received is value
            events.append('sent')
        finally:
            events.append('cleanup')
            yield value
            events.append('finished')
    return leaf()


g = factory(token)
alias = g
assert next(g) is token
exchange(json.dumps({'phase': 'dormant', 'events': events}))
assert alias is g and g.send(token) is token
exchange(json.dumps({'phase': 'cleanup', 'events': events}))
assert next(g, 'done') == 'done'
json.dumps({'events': events, 'identity': alias is g and token[0] is token})
