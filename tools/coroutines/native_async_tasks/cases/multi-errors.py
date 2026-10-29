import asyncio
import json


async def child():
    await exchange("A")
    raise ValueError("failure")


async def capture(task):
    try:
        await task
        assert False
    except ValueError as error:
        return error


async def main():
    task = asyncio.create_task(child())
    values = await asyncio.gather(capture(task), capture(task))
    assert values[0] is values[1] and values[0] is task.exception()
    await exchange("inspect")
    assert task.exception() is values[0]
    return json.dumps("ok")


asyncio.run(main())
