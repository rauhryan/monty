from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        await request(label)
    except asyncio.CancelledError as error:
        TRACE.append(['caught', label, str(error)])
        return label
    finally:
        await request('cleanup-' + label)
        TRACE.append(['cleaned', label])


async def parent():
    scope = Scope()
    async with scope:
        first = scope.spawn(operation('A'))
        second = scope.spawn(operation('B'))
        try:
            await first
        except asyncio.CancelledError as error:
            TRACE.append(['parent', str(error)])
        for label, handle in [('A', first), ('B', second)]:
            try:
                await handle
            except asyncio.CancelledError as error:
                TRACE.append(['joined-cancelled', label, str(error)])
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
