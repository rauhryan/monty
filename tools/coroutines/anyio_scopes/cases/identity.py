import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    scope = anyio.CancelScope()
    cycle = []
    error = ValueError(cycle)
    cycle.append(error)
    cycle.append(cycle)
    try:
        with scope:
            scope.cancel()
            scope.cancel()
            raise error
    except ValueError as caught:
        assert caught is error and caught.args[0] is cycle
    assert scope.cancel_called and not scope.cancelled_caught
    scope.cancel()
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
