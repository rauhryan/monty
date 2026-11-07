import asyncio
import json


async def child(label):
    return await exchange(label)


async def main():
    async with asyncio.TaskGroup() as group:
        first = group.create_task(child('A'))
        second = group.create_task(child('B'))
    exited = await exchange('parent-exit')
    return json.dumps([first.result(), second.result(), exited])


asyncio.run(main())
