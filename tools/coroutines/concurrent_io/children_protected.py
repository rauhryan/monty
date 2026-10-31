from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation():
    try:
        pass
    finally:
        reply = await request('cleanup')
        TRACE.append(reply)


async def parent():
    scope = Scope()
    async with scope:
        handle = scope.spawn(operation())
        await request('cancel')
        scope.cancel(handle)
        scope.cancel(handle)
        try:
            await handle
        except ValueError as error:
            TRACE.append(str(error))
    return json.dumps(
        {'trace': TRACE, 'errors': [str(error) for error in scope.errors]}
    )


parent()._native1()
