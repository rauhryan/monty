# ruff: noqa: F821
import asyncio
import json


def generator(token, pending):
    try:
        yield token
    finally:
        pending.append(exchange('unwinding'))


async def main():
    token = []
    token.append(token)
    pending = []
    error = ValueError(token)
    iterator = generator(token, pending)
    assert next(iterator) is token
    try:
        iterator.throw(error)
    except ValueError as caught:
        assert caught is error
        assert caught.args[0] is token
    else:
        assert False
    assert await pending.pop() == 'ok'
    assert next(iterator, 'closed') == 'closed'
    return json.dumps('ok')


asyncio.run(main())
