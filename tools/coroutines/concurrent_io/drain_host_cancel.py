from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        await request(label)
    finally:
        await request('cleanup-' + label)
        TRACE.append('drain-' + label)
        raise ValueError('scope-exit')


async def parent():
    scope = Scope()
    try:
        async with scope:
            scope.spawn(operation('A'))
            scope.spawn(operation('B'))
            await request('exit')
    except ValueError as error:
        TRACE.append(['caught', str(error)])
    finally:
        TRACE.append('parent-cleanup')
        await request('parent-cleanup')
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
