# ruff: noqa: F821
import asyncio
from contextlib import closing


async def main():
    pending = []

    class Resource:
        def close(self):
            pending.append(exchange('cleanup'))

    try:
        try:
            with closing(Resource()):
                await exchange('body')
        finally:
            assert await pending.pop() == 'ok'
    except asyncio.CancelledError:
        return 'suppressed'


asyncio.run(main())
