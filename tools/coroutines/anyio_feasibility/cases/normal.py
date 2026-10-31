import asyncio
import json
from typing import TYPE_CHECKING

import anyio

# Host supplies this callable as an execution input.
if TYPE_CHECKING:

    async def exchange(label: str) -> str:
        raise NotImplementedError


async def child(label):
    assert await exchange(label) == label.lower()


async def main():
    async with anyio.create_task_group() as group:
        start = group.start_soon
        assert start(child, "A") is None
        assert group.start_soon(child, "B") is None
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
