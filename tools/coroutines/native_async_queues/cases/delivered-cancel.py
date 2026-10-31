import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    async def worker():
        value = await q.get()
        assert value == "item"
        await exchange("delivered")

    task = asyncio.create_task(worker())
    await q.put("item")
    await exchange("control")
    assert task.cancel()
    try:
        await task
        assert False
    except ValueError:
        pass
    assert q.empty() and task.cancelled()
    await q.put("next")
    assert await q.get() == "next"
    return json.dumps("ok")


asyncio.run(main())
