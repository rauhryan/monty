# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    original = ValueError('unchanged')
    closed = []

    @asynccontextmanager
    async def manager():
        try:
            yield
        finally:
            assert await exchange('exit') == 'ok'
            closed.append(True)

    try:
        async with manager():
            raise original
    except ValueError as error:
        assert error is original
    else:
        assert False
    assert closed == [True]
    return json.dumps('ok')


asyncio.run(main())
