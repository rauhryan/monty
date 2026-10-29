from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def consumer(q, label):
    try:
        value = await asyncio._queue_get(q)
        TRACE.append([label, value])
    except ValueError as error:
        TRACE.append([label, str(error)])


async def parent():
    q = asyncio._queue(2)
    scope = Scope()
    async with scope:
        a = scope.spawn(consumer(q, 'A'))
        b = scope.spawn(consumer(q, 'B'))
        c = scope.spawn(consumer(q, 'C'))
        await request('empty')
        scope.cancel(b)
        try:
            await b
        except ValueError:
            pass
        await asyncio._queue_put(q, 'one')
        await asyncio._queue_put(q, 'two')
        await a
        await c
    return json.dumps(TRACE)


parent()._native1()
