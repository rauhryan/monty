# ruff: noqa: F821
import asyncio
import anyio
import json


async def child(queue, events):
    try:
        await queue.get()
    finally:
        with anyio.CancelScope(shield=True):
            await exchange('release')
            events.append('child')


async def main():
    queue = asyncio.Queue()
    events = []
    # Keep a genuine handled exception in the parked owner during child cleanup.
    error = ValueError('owner')
    try:
        raise error
    except ValueError as caught:
        async with anyio.create_task_group() as group:
            group.start_soon(child, queue, events)
            await exchange('control')
            group.cancel_scope.cancel()
        assert caught is error
    assert events == ['child'] and not group.cancel_scope.cancelled_caught
    await queue.put('fresh')
    assert await queue.get() == 'fresh'
    await exchange('after')
    return json.dumps('ok')


asyncio.run(main())
