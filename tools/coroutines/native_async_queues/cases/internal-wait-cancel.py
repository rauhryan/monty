import asyncio
import json


async def main():
    q = asyncio.Queue()
    try:
        await q.get()
        assert False
    finally:
        assert q.empty()
        await exchange("release")
    return json.dumps("unreachable")


asyncio.run(main())
