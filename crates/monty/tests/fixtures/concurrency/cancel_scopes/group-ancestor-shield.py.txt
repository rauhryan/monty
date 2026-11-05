import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

events = []
async def child():
    try:
        await exchange('child')
    finally:
        with anyio.CancelScope(shield=True):
            await exchange('child-release')
            events.append('child')
async def main():
    with anyio.CancelScope() as outer:
        async with anyio.create_task_group() as group:
            with anyio.CancelScope(shield=True):
                group.start_soon(child)
                await exchange('control')
                outer.cancel()
                await exchange('owner-protected')
            await exchange('forbidden')
    assert outer.cancelled_caught and events == ['child']
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
