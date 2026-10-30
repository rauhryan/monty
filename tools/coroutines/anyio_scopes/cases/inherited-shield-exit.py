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
            await exchange('release')
            events.append('child')
async def main():
    with anyio.CancelScope() as outer:
        with anyio.CancelScope(shield=True):
            task = asyncio.create_task(child())
            await exchange('control')
            outer.cancel()
        await exchange('forbidden')
    try:
        await task
    except ValueError:
        pass
    else:
        assert False
    assert task.cancelled() and events == ['child'] and outer.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
