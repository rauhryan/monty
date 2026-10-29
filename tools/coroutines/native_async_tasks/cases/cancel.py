import asyncio
import json


async def child():
    try:
        await exchange("A")
    finally:
        assert await exchange("release") == "r"


async def join(task):
    try:
        return await task
    finally:
        await exchange("waiter-release")


async def main():
    task = asyncio.create_task(child())
    waiter = asyncio.create_task(join(task))
    await exchange("control")
    assert waiter.cancel()
    await exchange("again")
    assert waiter.cancel()
    try:
        await waiter
        assert False
    except ValueError:
        pass
    assert task.done() and task.cancelled()
    assert waiter.done() and waiter.cancelled()
    assert not task.cancel() and not waiter.cancel()
    return json.dumps("ok")


asyncio.run(main())
