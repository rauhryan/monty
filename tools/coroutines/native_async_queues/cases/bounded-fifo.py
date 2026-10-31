import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    await q.put("seed")

    async def producer(value):
        await q.put(value)
        return value

    async with asyncio.TaskGroup() as group:
        a = group.create_task(producer("A"))
        b = group.create_task(producer("B"))
        await exchange("full")
        assert q.full() and not a.done() and not b.done()
        assert await q.get() == "seed"
        assert await q.get() == "A"
        assert await q.get() == "B"
    assert a.result() == "A" and b.result() == "B" and q.empty()
    await exchange("empty")
    return json.dumps("ok")


asyncio.run(main())
