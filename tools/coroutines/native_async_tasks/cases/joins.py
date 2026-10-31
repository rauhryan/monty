import asyncio
import json


async def child(label):
    return await exchange(label)


async def join(task):
    return await task


async def main():
    task = asyncio.create_task(child("A"))
    other = asyncio.create_task(child("B"))
    inspect = task.result
    assert not task.done() and not task.cancelled()
    for method in [task.result, task.exception]:
        try:
            method()
            assert False
        except RuntimeError:
            pass
    values = await asyncio.gather(join(task), join(task), task, task, other)
    assert values == ["a", "a", "a", "a", "b"]
    assert await task == "a" and await task == "a"
    assert inspect() == "a" and task.exception() is None
    assert task.done() and not task.cancelled() and not task.cancel()
    return json.dumps("ok")


asyncio.run(main())
