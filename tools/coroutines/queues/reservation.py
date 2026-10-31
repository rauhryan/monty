from __monty_host__ import host_coroutines
import json


def request(label):
    return (yield exchange(label))


async def observer(label):
    await request(label)


async def parent():
    q = asyncio._queue(1)
    scope = Scope()
    async with scope:
        witness = scope.spawn(observer('put-reserved'))
        await asyncio._queue_put(q, 'kept')
        await witness
    scope = Scope()
    async with scope:
        witness = scope.spawn(observer('get-reserved'))
        result = await asyncio._queue_get(q)
        await witness
    return json.dumps(result)


parent()._native1()
