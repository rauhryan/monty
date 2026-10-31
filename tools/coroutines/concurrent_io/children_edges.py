from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def never_started():
    TRACE.append('unexpected-start')
    await request('unexpected-call')


async def cleanup_failure():
    try:
        await request('body')
    finally:
        await request('cleanup')
        raise ValueError('cleanup-failure')


async def parent():
    scope = Scope()
    async with scope:
        handle = scope.spawn(never_started())
        scope.cancel(handle)
        try:
            await handle
        except ValueError as error:
            TRACE.append(str(error))
        scope.cancel(handle)
        for bad in [None, 1, []]:
            try:
                scope.cancel(bad)
            except ValueError:
                TRACE.append('unknown')
        scope.spawn(cleanup_failure())
        await request('exit')
    async with Scope() as other:
        try:
            other.cancel(handle)
        except ValueError:
            TRACE.append('foreign')
    return json.dumps(
        {'trace': TRACE, 'errors': [str(error) for error in scope.errors]}
    )


parent()._native1()
