import asyncio
import anyio
import json
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError
async def produce():
    return anyio.create_task_group()
async def main():
    task = asyncio.create_task(produce())
    original = await task
    alias = task
    await exchange('before')
    caught = []
    with anyio.CancelScope() as outer:
        outer.cancel()
        outer.cancel()
        with anyio.CancelScope(shield=True):
            assert await alias is original
            with anyio.CancelScope() as inner:
                inner.cancel()
                await alias
                return json.dumps('missed-inner')
            assert inner.cancelled_caught
            assert await task is original
        for i in range(2):
            try:
                await alias
            except ValueError as signal:
                caught.append(signal)
            else:
                return json.dumps('missed-outer')
        assert caught[0] is caught[1]
        await task
        return json.dumps('missed-final')
    assert outer.cancelled_caught and not task.cancelled()
    assert task.done() and await task is original
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
