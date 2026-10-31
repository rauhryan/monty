import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    scope = anyio.CancelScope()
    scope.cancel()
    assert scope.cancel_called and not scope.cancelled_caught
    with scope:
        await exchange('forbidden')
    assert scope.cancelled_caught
    scope.cancel()
    try:
        with scope:
            assert False
    except RuntimeError:
        pass
    empty = anyio.CancelScope()
    with empty:
        pass
    empty.cancel()
    assert empty.cancel_called and not empty.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
