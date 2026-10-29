from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def cleanup(label):
    value = await request('cleanup-' + label)
    TRACE.append([label, value])
    if label == 'B':
        raise ValueError('protected-B-failure')


async def group():
    try:
        await request('body')
    finally:
        await asyncio.gather(child(cleanup('A')), asyncio.gather(child(cleanup('B'))))


async def parent():
    scope = Scope()
    try:
        async with scope:
            scope.spawn(group())
            await request('exit')
    except ValueError as error:
        TRACE.append(['parent', str(error)])
    finally:
        TRACE.append('parent-cleanup')
        await request('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
