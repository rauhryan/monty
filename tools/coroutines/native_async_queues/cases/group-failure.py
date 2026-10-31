import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    async def blocked():
        try:
            await q.get()
            assert False
        finally:
            await exchange("child-release")

    async def bad():
        await exchange("fail")
        raise TypeError("failure")

    try:
        async with asyncio.TaskGroup() as group:
            waiter = group.create_task(blocked())
            failure = group.create_task(bad())
    except RuntimeError as error:
        assert error.args[0][0] is failure.exception()
        assert waiter.cancelled()
    assert q.empty()
    await q.put("kept")
    assert await q.get() == "kept"
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
