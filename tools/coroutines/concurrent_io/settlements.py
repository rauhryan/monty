from __monty_host__ import host_coroutines
import json


def request(label):
    return (yield exchange(label))


async def parent():
    async with Scope() as scope:
        first = scope.spawn(request('A'))
        second = scope.spawn(request('B'))
        a = await first
        b = await second
    return json.dumps([a, b])


parent()._native1()
