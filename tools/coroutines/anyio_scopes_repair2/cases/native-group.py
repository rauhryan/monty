import asyncio
import anyio
import json
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError
async def produce():
    return asyncio.TaskGroup()
async def main():
    task = asyncio.create_task(produce())
    original = await task
    await exchange('before')
    continued = False
    with anyio.CancelScope() as scope:
        scope.cancel()
        replay = await task
        continued = True
    await exchange('after')
    return json.dumps('ok' if scope.cancelled_caught and not continued else 'missed')
asyncio.run(main())
