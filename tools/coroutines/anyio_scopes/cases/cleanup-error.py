import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

cycle = []
error = TypeError(cycle)
cycle.append(cycle)
cycle.append(error)
async def child():
    try:
        await exchange('child')
    finally:
        with anyio.CancelScope(shield=True):
            await exchange('release')
        raise error
async def main():
    group = anyio.create_task_group()
    try:
        async with group:
            group.start_soon(child)
            await exchange('control')
            group.cancel_scope.cancel()
            await exchange('forbidden')
    except RuntimeError as caught:
        assert caught.args[0] == (error,)
        assert caught.args[0][0].args[0] is cycle
    else:
        assert False
    assert group.cancel_scope.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
