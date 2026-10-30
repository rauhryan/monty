import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

events = []
async def child():
    await exchange('child')
    events.append('child')
async def main():
    async with anyio.create_task_group() as group:
        with anyio.CancelScope(shield=True) as local:
            group.start_soon(child)
            await exchange('control')
            local.cancel()
            await exchange('forbidden')
        assert local.cancelled_caught and events == []
        await exchange('owner-after')
    assert events == ['child']
    assert not group.cancel_scope.cancel_called
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
