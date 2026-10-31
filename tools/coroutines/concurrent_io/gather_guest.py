from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        await request(label)
    except ValueError as error:
        TRACE.append([label, str(error)])
    finally:
        await request('cleanup-' + label)
        TRACE.append('cleaned-' + label)
        if label == 'A':
            raise ValueError('cleanup-A-failure')


async def branch():
    await asyncio.gather(child(operation('B')))


async def group():
    await asyncio.gather(child(operation('A')), asyncio.gather(child(branch())))


async def parent():
    scope = Scope()
    async with scope:
        handle = scope.spawn(group())
        unrelated = scope.spawn(request('C'))
        await request('cancel')
        scope.cancel(handle)
        try:
            await handle
        except ValueError as error:
            TRACE.append(['parent', str(error)])
        TRACE.append(['unrelated', await unrelated])
    TRACE.append('parent-cleanup')
    await request('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
