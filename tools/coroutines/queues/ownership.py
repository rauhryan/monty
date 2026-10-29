from __monty_host__ import host_coroutines


def request(label):
    return (yield exchange(label))


async def blocked(q, value, put):
    try:
        if put:
            await asyncio._queue_put(q, value)
        else:
            await asyncio._queue_get(q)
    finally:
        await request('cleanup')


async def parent():
    q = asyncio._queue(1)
    empty = asyncio._queue(1)
    cycle = [q, empty]
    cycle.append(cycle)
    await asyncio._queue_put(q, cycle)
    async with Scope() as scope:
        scope.spawn(blocked(q, cycle, True))
        scope.spawn(blocked(empty, cycle, False))
        try:
            await request('body')
        except ValueError:
            pass
    assert await asyncio._queue_get(q) is cycle
    q = empty = cycle = scope = None
    await request('parent-cleanup')


parent()._native1()
