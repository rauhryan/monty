import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def controller(scope):
    await exchange('control')
    scope.cancel()
async def main():
    with anyio.CancelScope() as scope:
        async with anyio.create_task_group() as group:
            group.start_soon(controller, scope)
            await asyncio.gather(exchange('A'), exchange('B'))
    assert scope.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
