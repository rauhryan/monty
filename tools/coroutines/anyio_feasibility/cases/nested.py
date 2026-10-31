import asyncio
import json
from typing import TYPE_CHECKING

import anyio

# Host supplies this callable as an execution input.
if TYPE_CHECKING:

    async def exchange(label: str) -> str:
        raise NotImplementedError


async def main():
    try:
        with anyio.CancelScope() as outer:
            with anyio.CancelScope(shield=True):
                outer.cancel()
                assert await exchange("shielded") == "protected"
            await exchange("must-not-run")
        assert outer.cancelled_caught
    except NotImplementedError as error:
        return json.dumps({"unsupported": str(error)})
    await exchange("after")
    return json.dumps("ok")


asyncio.run(main())
