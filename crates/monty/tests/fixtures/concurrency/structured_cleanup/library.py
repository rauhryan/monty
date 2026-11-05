from __monty_host__ import host_coroutines

"""Cooperative async scope library; assembled as source, never an installed import."""
# ruff: noqa: F401, F821


def request(message):
    """Yield exactly one control/effect request to the selecting host."""
    return (yield message)


class Timebox:
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

    async def spawn(self, label, mode):
        return await request({'kind': 'spawn', 'scope': self.scope['id'], 'label': label, 'mode': mode})

    async def join(self, child):
        return await request({'kind': 'join', 'scope': self.scope['id'], 'child': child})

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


async def parent_body(state, task, mode):
    try:
        for once in [0]:
            async with Timebox(state, task, 50, mode == 'suppress') as tg:
                tg.defer('parent-arbitrary', 'P')
                if mode == 'nested':
                    async with Timebox(state, task, 25) as inner:
                        inner.defer('inner-arbitrary', 'I')
                        await inner.operation(['nested-work'])
                first = await tg.spawn('a', mode)
                second = await tg.spawn('b', mode)
                if mode == 'early':
                    return 'early'
                if mode in ('early-live', 'error', 'suppress', 'break'):
                    await tg.operation(['parent-work'])
                    if mode in ('error', 'suppress'):
                        raise ValueError('body-original')
                    if mode == 'break':
                        break
                    return 'early-live'
                one = await tg.join(first)
                if one['kind'] == 'error' and mode != 'ignore-error':
                    raise ValueError('child:failed' if one['cause'] is None else 'child:' + one['cause'])
                two = await tg.join(second)
                return [one, two]
        return 'after-scope'
    except ValueError:
        if mode == 'catch':
            return 'caught-parent-success'
        raise


async def child_body(state, task, mode):
    async with Timebox(state, task, 40) as tg:
        hold = await tg.operation(['acquire', task['label']])
        tg.defer('arbitrary-child', hold)
        tg.defer('another-name', hold)
        try:
            return await tg.operation(['use', hold])
        except ValueError:
            if mode == 'catch':
                return 'caught-child-success'
            raise
