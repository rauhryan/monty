import asyncio
import json


async def child(label):
    try:
        await exchange(label)
    finally:
        await exchange(label + "-release")


async def join(task):
    return await task


async def owner():
    try:
        a = asyncio.create_task(child("A"))
        b = asyncio.create_task(child("B"))
        await asyncio.gather(join(a), join(b))
    finally:
        await exchange("owner-release")


async def main():
    task = asyncio.create_task(owner())
    await exchange("control")
    assert task.cancel()
    try:
        await task
        assert False
    except ValueError:
        pass
    assert task.cancelled()
    return json.dumps("ok")


asyncio.run(main())
