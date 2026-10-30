import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def forbidden_child():
    assert False
async def main():
    with anyio.CancelScope() as guest:
        try:
            await exchange('body')
        except ValueError as error:
            assert str(error) == 'FIRST'
        guest.cancel()
        try:
            await exchange('forbidden')
        finally:
            with anyio.CancelScope(shield=True):
                await exchange('cleanup')
    assert guest.cancel_called and guest.cancelled_caught
    try:
        asyncio.create_task(forbidden_child())
    except ValueError:
        pass
    else:
        assert False
    raise ValueError('FIRST')
asyncio.run(main())
