# ruff: noqa: F821
import asyncio
import json
from contextlib import contextmanager


@contextmanager
def wrapped():
    try:
        yield
    except BaseException as error:
        raise RuntimeError('wrapped') from error


async def main():
    token = []
    token.append(token)
    stop = StopIteration(token)
    try:
        with wrapped():
            raise stop
    except StopIteration as caught:
        assert caught is stop
        assert await exchange('stop') == 'ok'
        assert caught.value is token
    else:
        assert False
    original = ValueError(token)
    try:
        with wrapped():
            raise original
    except RuntimeError as replacement:
        assert replacement.__cause__ is original
        assert replacement.__context__ is original
        assert await exchange('replacement') == 'ok'
        assert replacement.__cause__.args[0] is token
    else:
        assert False
    return json.dumps('ok')


asyncio.run(main())
