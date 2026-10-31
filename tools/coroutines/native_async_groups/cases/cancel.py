import asyncio
import json


async def child(label):
    try:
        await exchange(label)
    finally:
        await exchange(label + "-release")


async def owner():
    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(child("A"))
            group.create_task(child("B"))
            await exchange("body")
    finally:
        await exchange("owner-release")


async def main():
    task = asyncio.create_task(owner())
    await exchange("control")
    task.cancel()
    await exchange("repeat")
    task.cancel()
    try:
        await task
    except ValueError:
        assert task.cancelled()
    return json.dumps("ok")


asyncio.run(main())
