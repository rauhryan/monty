from __monty_host__ import host_coroutines

"""Cooperative async scope library; assembled as source, never an installed import."""
# ruff: noqa: F401, F821


def request(message):
    """Yield exactly one control/effect request to the supplied coordinator."""
    return (yield message)


class Scope:
    """Own children and inert cleanup descriptors until an explicit drain barrier."""

    def __init__(self, state, task, deadline, suppress=False):
        self.state = state
        self.task = task
        self.deadline = deadline
        self.suppress = suppress
        self.scope = None

    async def __aenter__(self):
        self.scope = open_scope(self.state, self.task, self.deadline)
        return self

    def defer(self, operation, handle):
        """Store data only: registration cannot start an external effect."""
        assert self.scope['phase'] == 'body'
        self.scope['deferred'].append([operation, handle])

    async def operation(self, payload):
        return await request({'kind': 'operation', 'scope': self.scope['id'], 'payload': payload})

    async def spawn(self, factory, label, mode=None):
        return await request(
            {'kind': 'spawn', 'scope': self.scope['id'], 'label': label, 'mode': mode, 'factory': factory}
        )

    async def join(self, child):
        return await request({'kind': 'join', 'scope': self.scope['id'], 'child': child})

    async def cancel(self, child, reason='guest'):
        return await request({'kind': 'task-cancel', 'scope': self.scope['id'], 'child': child, 'reason': reason})

    async def checkpoint(self):
        return await request({'kind': 'checkpoint', 'scope': self.scope['id']})

    async def put(self, queue, value):
        return await request({'kind': 'put', 'scope': self.scope['id'], 'queue': queue, 'value': value})

    async def get(self, queue):
        return await request({'kind': 'get', 'scope': self.scope['id'], 'queue': queue})

    async def __aexit__(self, typ, value, traceback):
        scope = self.scope
        scope['cause'] = None if typ is None else str(value)
        scope['phase'] = 'closing'
        self.task['phase'] = 'closing'
        await request({'kind': 'drain', 'scope': scope['id']})
        scope['phase'] = 'resource-cleanup'
        self.task['phase'] = 'resource-cleanup'
        scope['cursor'] = len(scope['deferred'])
        while scope['cursor'] > 0:
            scope['cursor'] -= 1
            try:
                await self.operation(scope['deferred'][scope['cursor']])
            except Exception as error:
                scope['failures'].append(str(error))
        close_scope(self.state, self.task, scope)
        return self.suppress
