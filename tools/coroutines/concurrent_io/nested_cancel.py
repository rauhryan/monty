from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def leaf():
    try:
        await request('leaf')
    except ValueError as error:
        TRACE.append(['leaf-caught', str(error)])
    finally:
        await request('leaf-cleanup')
        TRACE.append('leaf-cleaned')


async def owner():
    scope = Scope()
    try:
        async with scope:
            handle = scope.spawn(leaf())
            await handle
    except ValueError as error:
        TRACE.append(['owner-caught', str(error)])
    finally:
        TRACE.append('owner-cleanup')
        await request('owner-cleanup')


async def parent():
    scope = Scope()
    async with scope:
        handle = scope.spawn(owner())
        try:
            await handle
        except ValueError as error:
            TRACE.append(['parent-caught', str(error)])
    TRACE.append('parent-cleanup')
    return json.dumps(TRACE)


parent()._native1()
