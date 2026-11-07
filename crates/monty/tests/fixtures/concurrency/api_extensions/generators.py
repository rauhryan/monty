# ruff: noqa: F821
import asyncio
import json


def leaf(token):
    yield token
    yield token
    return 'ignored'


def delegate(token):
    return (yield from leaf(token))


def host_generator():
    yield exchange('iteration')
    yield exchange('iteration-again')


async def main():
    token = []
    values = []
    for value in delegate(token):
        values.append(value)
    else:
        values.append('exhausted')
    assert values == [token, token, 'exhausted']
    assert values[0] is values[1]
    gen = leaf(token)
    for value in gen:
        break
    assert next(gen) is token
    assert next(gen, 'end') == 'end'
    for value in gen:
        assert False
    assert [value for value in leaf(3)] == [3, 3]
    for future in host_generator():
        assert await future == 'ok'
    return json.dumps('ok')


asyncio.run(main())
