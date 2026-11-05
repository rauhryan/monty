import asyncio
import json


async def child(label):
    return await exchange(label)


async def main():
    first = asyncio.create_task(child('A'))
    second = asyncio.create_task(child('B'))
    values = [await first, await second]
    return json.dumps([values, first.result(), second.result()])


asyncio.run(main())
