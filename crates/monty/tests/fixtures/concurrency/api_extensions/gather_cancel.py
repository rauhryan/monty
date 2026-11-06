# ruff: noqa: F821
import asyncio
import json


async def child(queue):
    try:
        await queue.get()
    finally:
        await exchange('cleanup')


async def collecting(queue):
    await asyncio.gather(child(queue), return_exceptions=True)
    assert False, 'cancellation of gather must not become a result'


async def main():
    task = asyncio.create_task(collecting(asyncio.Queue()))
    await exchange('control')
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass
    else:
        assert False
    assert task.cancelled()
    return json.dumps('ok')


asyncio.run(main())
