# ruff: noqa: F821
import asyncio
import json


class Source:
    def __init__(self, label, pending, token, error, mode):
        self.label = label
        self.pending = pending
        self.token = token
        self.error = error
        self.mode = mode
        self.n = 0

    def __iter__(self):
        self.pending.append(exchange(self.label + '-iter'))
        return self

    def __next__(self):
        self.n += 1
        self.pending.append(exchange(self.label + '-' + str(self.n)))
        if self.mode == 'empty' or (self.mode == 'retry' and self.n == 3):
            raise StopIteration
        if self.mode == 'retry' and self.n == 1:
            raise self.error
        return self.token


async def main():
    token = []
    token.append(token)
    pending = []
    error = ValueError(token)
    z = zip(
        Source('a', pending, token, error, 'full'),
        Source('b', pending, token, error, 'retry'),
        strict=True,
    )
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    try:
        next(z)
    except ValueError as caught:
        assert caught is error
    else:
        assert False
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    row = next(z)
    assert row[0] is token and row[1] is token
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    try:
        next(z)
    except ValueError as caught:
        assert str(caught) == 'zip() argument 2 is shorter than argument 1'
        assert caught.__context__ is None
    else:
        assert False
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    z = zip(
        Source('c', pending, token, error, 'empty'),
        Source('d', pending, token, error, 'full'),
        strict=True,
    )
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    try:
        next(z)
    except ValueError as caught:
        assert str(caught) == 'zip() argument 2 is longer than argument 1'
        assert caught.__context__ is None
    else:
        assert False
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    return json.dumps('ok')


asyncio.run(main())
