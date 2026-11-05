import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    await q.put("seed")

    async def blocked():
        try:
            await q.put("abandoned")
        finally:
            await exchange("child-release")

    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(blocked())
            await exchange("body")
    finally:
        assert q.qsize() == 1
        assert await q.get() == "seed"
        await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
