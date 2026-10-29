from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def blocked(q, label):
    try:
        if label == 'put':
            await asyncio._queue_put(q, 'undelivered')
        else:
            await asyncio._queue_get(q)
    except ValueError as error:
        TRACE.append([label, str(error)])
    finally:
        value = await request('cleanup-' + label)
        TRACE.append([label, value])


async def parent():
    full = asyncio._queue(1)
    empty = asyncio._queue(1)
    await asyncio._queue_put(full, 'seed')
    scope = Scope()
    async with scope:
        scope.spawn(blocked(full, 'put'))
        scope.spawn(blocked(empty, 'get'))
        try:
            await request('exit')
        except ValueError as error:
            TRACE.append(['parent', str(error)])
    await request('parent-cleanup')
    TRACE.append('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
