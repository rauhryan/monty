from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def producer(q, value):
    await asyncio._queue_put(q, value)
    TRACE.append(value)


async def parent():
    q = asyncio._queue(1)
    await asyncio._queue_put(q, 'seed')
    scope = Scope()
    async with scope:
        a = scope.spawn(producer(q, 'A'))
        b = scope.spawn(producer(q, 'B'))
        await request('full')
        scope.cancel(a)
        try:
            await a
        except ValueError:
            pass
        values = [await asyncio._queue_get(q), await asyncio._queue_get(q)]
        await b
    return json.dumps({'values': values, 'puts': TRACE})


parent()._native1()
