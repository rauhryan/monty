import asyncio
import json


async def child(label):
    try:
        await exchange(label)
    finally:
        await exchange(label + "-release")


async def main():
    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(child("A"))
            group.create_task(child("B"))
            await exchange("body")
    finally:
        await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
