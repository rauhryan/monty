import asyncio
import json


async def child(label):
    return await exchange(label)


async def main():
    async with asyncio.TaskGroup() as outer:
        first = outer.create_task(child("A"))
        async with asyncio.TaskGroup() as inner:
            second = inner.create_task(child("B"))
        assert second.result() == "b"
        await exchange("after-inner")
    assert first.result() == "a"
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
