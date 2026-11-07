# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    events = []

    @asynccontextmanager
    async def manager():
        try:
            yield
        finally:
            assert await exchange('cleanup') == 'ok'
            events.append('closed')

    try:
        async with manager():
            await exchange('body')
    except Exception:
        assert False
    except asyncio.CancelledError:
        assert events == ['closed']
        return 'suppressed'
    return json.dumps('ok')


asyncio.run(main())
