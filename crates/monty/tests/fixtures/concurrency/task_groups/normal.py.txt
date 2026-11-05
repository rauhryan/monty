import asyncio
import json


async def child(label):
    return await exchange(label)


async def nested():
    async with asyncio.TaskGroup() as inner:
        handle = inner.create_task(child("B"))
    return handle.result()


async def main():
    group = asyncio.TaskGroup()
    async with group:
        method = group.create_task
        first = method(child("A"))
        second = group.create_task(nested())
        group.create_task(child("discarded"))
    assert first.result() == "a" and second.result() == "b"
    assert await asyncio.gather(first, second, first) == ["a", "b", "a"]
    try:
        async with group:
            assert False
    except RuntimeError:
        pass
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
