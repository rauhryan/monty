import asyncio
import json
from typing import TYPE_CHECKING

import anyio

# Host supplies this callable as an execution input.
if TYPE_CHECKING:

    async def exchange(label: str) -> str:
        raise NotImplementedError


async def first():
    await exchange("A")
    raise ValueError("first")


async def sibling():
    try:
        await exchange("B")
    finally:
        raise TypeError("second")


async def main():
    try:
        async with anyio.create_task_group() as group:
            group.start_soon(first)
            group.start_soon(sibling)
            await exchange("body")
            assert False, "failure must interrupt the body"
    except Exception as error:
        try:
            errors = error.exceptions
        except AttributeError:
            errors = error.args[0]
        assert len(errors) == 2
        assert (
            isinstance(errors[0], ValueError)
            and repr(errors[0]) == "ValueError('first')"
        )
        assert (
            isinstance(errors[1], TypeError)
            and repr(errors[1]) == "TypeError('second')"
        )
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
