import asyncio
import json

queue = asyncio.Queue(1)


async def first():
    await queue.put("seed")
    task = asyncio.create_task(queue.put("discarded"))
    await exchange("first")
    return "one"


async def second():
    assert queue.qsize() == 1
    assert await queue.get() == "seed"
    async with asyncio.TaskGroup() as group:
        get = group.create_task(queue.get())
        group.create_task(queue.put("next"))
    assert get.result() == "next" and queue.empty()
    await exchange("second")
    return json.dumps("ok")


assert asyncio.run(first()) == "one"
asyncio.run(second())
