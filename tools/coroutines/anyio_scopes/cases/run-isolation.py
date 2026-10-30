import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

saved = []
async def setup():
    scope = anyio.CancelScope()
    with scope:
        pass
    saved.append(scope)
asyncio.run(setup())
async def main():
    scope = saved[0]
    try:
        scope.cancel()
        assert False
    except RuntimeError:
        pass
    try:
        with scope:
            assert False
    except RuntimeError:
        pass
    assert not scope.cancel_called and not scope.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
