import asyncio
import json


async def child(label):
    value = await exchange(label)
    if label == "B":
        raise ValueError(value)
    return value


async def creator():
    return asyncio.create_task(child("A"))


async def main():
    task = await asyncio.create_task(creator())
    asyncio.create_task(child("B"))
    assert await task == "a"
    assert task.result() == "a"
    await exchange("finish")
    return json.dumps("ok")


asyncio.run(main())
