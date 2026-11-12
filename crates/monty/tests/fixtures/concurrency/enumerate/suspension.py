# ruff: noqa: F821
import asyncio
import json


class Source:
    def __init__(self, token, pending, error):
        self.token = token
        self.pending = pending
        self.error = error
        self.n = 0

    def __iter__(self):
        self.pending.append(exchange('iter'))
        return self

    def __next__(self):
        self.n += 1
        self.pending.append(exchange('next-' + str(self.n)))
        if self.n == 1:
            raise self.error
        if self.n == 3:
            raise StopIteration
        return self.token


async def main():
    token = []
    token.append(token)
    pending = []
    error = ValueError(token)
    e = enumerate(Source(token, pending, error), 2**100)
    assert await pending.pop() == 'ok'
    try:
        next(e)
    except ValueError as caught:
        assert caught is error
    else:
        assert False
    assert await pending.pop() == 'ok'
    index, value = next(e)
    assert index == 2**100 and value is token
    assert await pending.pop() == 'ok'
    assert next(e, None) is None
    assert await pending.pop() == 'ok'

    def values():
        yield exchange('generator')

    for index, value in enumerate(values(), 9):
        assert index == 9 and await value == 'ok'
    return json.dumps('ok')


asyncio.run(main())
