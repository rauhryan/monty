import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    try:
        with anyio.CancelScope(shield=True) as scope:
            try:
                await exchange('body')
            finally:
                with anyio.CancelScope(shield=True):
                    await exchange('release')
    except ValueError as error:
        assert str(error) == 'FIRST'
        assert not scope.cancel_called and not scope.cancelled_caught
        raise
    assert not scope.cancel_called and not scope.cancelled_caught
    return json.dumps('forbidden-success')
asyncio.run(main())
