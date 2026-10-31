import asyncio
import json


async def child():
    try:
        await exchange("A")
    except ValueError:
        assert await exchange("recover") == "r"
        return 42


async def join(task):
    return await task


async def main():
    task = asyncio.create_task(child())
    waiter = asyncio.create_task(join(task))
    await exchange("control")
    assert waiter.cancel()
    assert await waiter == 42
    assert await task == 42
    assert not waiter.cancelled() and not task.cancelled()
    return json.dumps("ok")


asyncio.run(main())
