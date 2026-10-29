import asyncio
import json


async def fail():
    await exchange("A")
    raise ValueError("first")


async def body_child(label):
    try:
        await exchange(label)
    finally:
        await exchange(label + "-release")
        raise TypeError(label + "-error")


async def main():
    outside = asyncio.create_task(body_child("K"))
    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(fail())
            await asyncio.gather(body_child("B"), asyncio.gather(outside, outside))
    except RuntimeError as error:
        errors = error.args[0]
        assert sorted(str(item) for item in errors) == ["B-error", "K-error", "first"]
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
