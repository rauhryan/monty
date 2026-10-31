import asyncio
import json
from typing import TYPE_CHECKING

import anyio

# Host supplies this callable as an execution input.
if TYPE_CHECKING:

    async def exchange(label: str) -> str:
        raise NotImplementedError


async def child():
    try:
        await exchange("A")
    finally:
        assert await exchange("child-release") == "released"


async def main():
    try:
        async with anyio.create_task_group() as group:
            group.start_soon(child)
            await exchange("body")
    finally:
        assert await exchange("parent-release") == "released"
    return json.dumps("must-not-complete")


asyncio.run(main())
