import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    with anyio.CancelScope() as outer:
        outer.cancel()
        with anyio.CancelScope(shield=True) as shield:
            await exchange('protected')
            shield.cancel()
            try:
                await exchange('forbidden-inner')
            except BaseException as error:
                assert isinstance(error, anyio.get_cancelled_exc_class())
        assert not shield.cancelled_caught
        await exchange('forbidden-outer')
    assert outer.cancelled_caught
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
