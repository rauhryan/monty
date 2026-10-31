from __monty_host__ import host_coroutines
import json

# Minimal authoring shim, deliberately no task registry, runnable queue or pump.
# Only nested single-task scopes are supported in this slice.
PUBLIC = []
NEXT = 0


def request(payload):
    global NEXT
    operation = 'O' + str(NEXT)
    NEXT += 1
    PUBLIC.append(['request', operation, payload])
    messages = json.loads((yield exchange(json.dumps({'events': PUBLIC, 'trace': TRACE, 'result': None}))))
    assert len(messages) == 1, 'native1 supports one settlement per boundary'
    message = messages[0]
    assert message['run'] == 'C1' and message['kind'] == 'reply'
    assert message['operation'] == operation
    assert message['outcome'] in ('value', 'error')
    PUBLIC.append(['settle', operation, message['outcome'], message['value']])
    if message['outcome'] == 'error':
        raise ValueError(message['value'])
    return message['value']


class Scope:
    def __init__(self, state, task, parent):
        self.deferred = []

    async def __aenter__(self):
        return self

    def defer(self, kind, value):
        self.deferred.append([kind, value])

    async def operation(self, payload):
        return await request(payload)

    async def __aexit__(self, typ, error, tb):
        for payload in reversed(self.deferred):
            await request(payload)
        return False
