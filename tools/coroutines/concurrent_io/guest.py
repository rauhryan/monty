from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        return [label, await request(label)]
    finally:
        TRACE.append(label)


async def parent():
    values = await asyncio.gather(child(operation('A')), child(operation('B')))
    return json.dumps({'settled': TRACE, 'joined': values})


parent()._native1()
