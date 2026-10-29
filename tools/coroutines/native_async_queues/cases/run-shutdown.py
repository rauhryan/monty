import asyncio
import json


async def worker(q):
    try:
        await q.get()
        assert False
    finally:
        await exchange("child-release")


async def main():
    q = asyncio.Queue()
    asyncio.create_task(worker(q))
    await exchange("control")
    return json.dumps("ok")


asyncio.run(main())
