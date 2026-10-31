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
        with anyio.CancelScope() as scope:
            scope.cancel()
            for label in ["first", "second"]:
                try:
                    await exchange(label)
                except BaseException as error:
                    assert isinstance(error, anyio.get_cancelled_exc_class())
            with anyio.CancelScope(shield=True):
                await exchange("shielded")
        assert scope.cancel_called and not scope.cancelled_caught
    except NotImplementedError as error:
        return json.dumps({"unsupported": str(error)})
    await exchange("after")
    return json.dumps("ok")


asyncio.run(main())
