from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        await request(label)
    except ValueError:
        TRACE.append('cancel-' + label)
    finally:
        await request('cleanup-' + label)
        TRACE.append('drain-' + label)


async def parent():
    scope = Scope()
    async with scope:
        scope.spawn(operation('A'))
        scope.spawn(operation('B'))
        await request('exit')
    TRACE.append('parent-cleanup')
    return json.dumps(
        {'trace': TRACE, 'errors': [str(error) for error in scope.errors]}
    )


parent()._native1()
