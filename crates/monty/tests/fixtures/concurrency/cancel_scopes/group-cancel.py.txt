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
    async with anyio.create_task_group() as group:
        group.start_soon(child)
        await exchange('control')
        group.cancel_scope.cancel()
        await exchange('forbidden')
    assert group.cancel_scope.cancel_called and group.cancel_scope.cancelled_caught
    assert events == ['child']
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
