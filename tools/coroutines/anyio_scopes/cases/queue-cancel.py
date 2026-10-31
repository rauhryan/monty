import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

events = []
async def child(q):
    try:
        await q.get()
    finally:
        with anyio.CancelScope(shield=True):
            await exchange('release')
            events.append('child')
async def main():
    q = asyncio.Queue()
    async with anyio.create_task_group() as group:
        group.start_soon(child, q)
        await exchange('control')
        group.cancel_scope.cancel()
    assert events == ['child'] and not group.cancel_scope.cancelled_caught
    await q.put('fresh')
    assert await q.get() == 'fresh'
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
