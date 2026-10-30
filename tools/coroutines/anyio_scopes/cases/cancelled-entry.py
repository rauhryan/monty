import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    with anyio.CancelScope() as outer:
        outer.cancel()
        async with anyio.create_task_group() as group:
            await exchange('forbidden')
    assert outer.cancelled_caught
    assert not group.cancel_scope.cancel_called and not group.cancel_scope.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
