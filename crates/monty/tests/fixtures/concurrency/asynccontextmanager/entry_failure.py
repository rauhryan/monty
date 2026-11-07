# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    original = ValueError('entry')

    @asynccontextmanager
    async def manager():
        assert await exchange('entry') == 'ok'
        raise original
        yield

    try:
        async with manager():
            assert False
    except ValueError as error:
        assert error is original
    else:
        assert False
    return json.dumps('ok')


asyncio.run(main())
