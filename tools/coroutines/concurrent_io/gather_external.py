from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def parent():
    mode = await request('mode')
    scope = Scope()
    async with scope:
        handle = asyncio._spawn(direct_gather())
        try:
            await request('exit')
        except ValueError as error:
            TRACE.append(['host', str(error)])
        if mode == 'cancel':
            scope.cancel(handle)
        try:
            TRACE.append(await handle)
        except ValueError as error:
            TRACE.append(str(error))
    TRACE.append('parent-cleanup')
    await request('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
