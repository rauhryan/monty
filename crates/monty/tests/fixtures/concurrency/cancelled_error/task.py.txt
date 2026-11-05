# ruff: noqa: F821
import asyncio
import json


async def child(queue):
    try:
        await queue.get()
    except Exception:
        assert False, "ordinary handler caught task cancellation"
    finally:
        await exchange("cleanup")


async def main():
    task = asyncio.create_task(child(asyncio.Queue()))
    await exchange("control")
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass
    else:
        assert False
    assert task.cancelled()
    for inspect in [task.result, task.exception]:
        try:
            inspect()
        except asyncio.CancelledError:
            pass
        else:
            assert False
    return json.dumps("ok")


asyncio.run(main())
