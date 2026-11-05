"""Fixed concurrency, scalar application outcome, repeated producer/worker lifecycles."""
# ruff: noqa: F821


async def producer(state, task, mode):
    async with Scope(state, task, None) as scope:
        await scope.put('work', mode)
        return None


async def worker(state, task, mode):
    async with Scope(state, task, None) as scope:
        if mode == 'cancel':
            scope.defer('close', 'worker')
            scope.defer('audit', 'worker')
        value = await scope.get('work')
        if mode == 'cancel':
            await scope.operation(['hold', value])
        return value


async def parent_body(state, task, mode):
    state['queues'] = {'work': {'items': [], 'get': [], 'put': []}}
    state['cycles'] = 0
    async with Scope(state, task, None) as scope:
        scope.defer('close', 'parent')
        total = 0
        for i in range(1500):
            if i > 0:
                await scope.cancel(a, 'completed-producer')
                await scope.cancel(b, 'completed-worker')
            a = await scope.spawn(producer, 'producer', i)
            b = await scope.spawn(worker, 'worker', 'cancel' if mode == 'cancel' and i == 1000 else None)
            await scope.join(a)
            result = await scope.join(b)
            assert result['kind'] == 'value'
            total += result['value']
            state['cycles'] += 1
            if state['cycles'] % 50 == 0:
                await scope.operation(['sample', state['cycles']])
        return total


def pump(state, messages):
    turn(state, messages, 4)
    for _ in range(10000):
        assert len(state['tasks']) <= 3
        assert len(state['scopes']) <= 3
        assert len(state['events']) <= 64
        if not state['ready']:
            return
        turn(state, [], 4)
    assert False, 'stress pump did not reach an external boundary'


def measurement(state):
    result = observe(state)
    result['cycles'] = state['cycles']
    result['reclaimed_tasks'] = state['reclaimed_tasks']
    result['created_tasks'] = state['next_task']
    result['seen_count'] = len(state['seen'])
    return result
