import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

events = []
async def leaf():
    try:
        await exchange('leaf')
    finally:
        with anyio.CancelScope(shield=True):
            await exchange('leaf-release')
            events.append('leaf')
async def child():
    try:
        async with anyio.create_task_group() as inner:
            inner.start_soon(leaf)
    finally:
        with anyio.CancelScope(shield=True):
            assert events == ['leaf']
            await exchange('parent-release')
            events.append('parent')
async def main():
    async with anyio.create_task_group() as outer:
        outer.start_soon(child)
        await exchange('control')
        outer.cancel_scope.cancel()
        await exchange('forbidden')
    assert events == ['leaf', 'parent']
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
