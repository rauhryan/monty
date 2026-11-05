from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        await request(label)
        if label == 'A':
            raise ValueError('first-error')
    finally:
        await request('cleanup-' + label)
        TRACE.append('cleaned-' + label)
        if label == 'B':
            raise ValueError('cleanup-B-failure')


async def group():
    await asyncio.gather(child(operation('A')), child(operation('B')))


async def parent():
    scope = Scope()
    async with scope:
        handle = scope.spawn(group())
        try:
            await handle
        except ValueError as error:
            TRACE.append(str(error))
        await request('continue')
    TRACE.append('parent-cleanup')
    await request('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
