import asyncio
import json
from typing import TYPE_CHECKING

# Host supplies this callable as an execution input.
if TYPE_CHECKING:

    async def exchange(label: str) -> str:
        raise NotImplementedError


async def child():
    try:
        await exchange("A")
    except BaseException:
        pass
    # This second wait is admitted: native task cancellation is coalesced, not
    # AnyIO level cancellation from a still-cancelled lexical scope.
    assert await exchange("after-catch") == "reply"


async def main():
    task = asyncio.create_task(child())
    await exchange("control")
    assert task.cancel()
    await task
    assert task.done() and not task.cancelled()
    return json.dumps("coalesced")


asyncio.run(main())
