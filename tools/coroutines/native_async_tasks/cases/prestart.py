import asyncio
import json


async def never():
    assert False, "cancelled before first activation"


async def self_join():
    try:
        await target
        assert False
    except RuntimeError:
        return 42


async def main():
    global target
    task = asyncio.create_task(never())
    assert task.cancel() and task.cancel()
    await exchange("inspect")
    assert task.done() and task.cancelled() and not task.cancel()
    for method in [task.result, task.exception]:
        try:
            method()
            assert False
        except ValueError:
            pass
    for _ in range(2):
        try:
            await task
            assert False
        except ValueError:
            pass
    target = asyncio.create_task(self_join())
    assert await target == 42
    return json.dumps("ok")


asyncio.run(main())
