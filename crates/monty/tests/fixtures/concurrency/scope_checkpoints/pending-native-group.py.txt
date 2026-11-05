import asyncio
import anyio
import json
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError
async def produce():
    await exchange('child')
    return asyncio.TaskGroup()
async def main():
    task = asyncio.create_task(produce())
    with anyio.CancelScope() as scope:
        await exchange('control')
        scope.cancel()
        scope.cancel()
        await task
        return json.dumps('missed-pending')
    assert scope.cancelled_caught and not task.done()
    result = await task
    assert task.done() and not task.cancelled()
    assert await task is result
    with anyio.CancelScope() as again:
        again.cancel()
        await task
        return json.dumps('missed-ready')
    assert again.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
