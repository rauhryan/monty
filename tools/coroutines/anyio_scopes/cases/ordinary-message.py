import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    error = ValueError('cancelled by AnyIO CancelScope')
    scope = anyio.CancelScope()
    try:
        with scope:
            scope.cancel()
            raise error
    except ValueError as caught:
        assert caught is error
    else:
        assert False
    assert scope.cancel_called and not scope.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
