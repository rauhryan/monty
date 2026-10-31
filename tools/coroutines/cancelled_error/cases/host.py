# ruff: noqa: F821
import asyncio
import anyio
import json


async def main():
    assert anyio.get_cancelled_exc_class() is asyncio.CancelledError
    assert isinstance(asyncio.CancelledError(), BaseException)
    assert not isinstance(asyncio.CancelledError(), Exception)
    try:
        try:
            await exchange("body")
        except ValueError:
            assert False, "validation handler caught cancellation"
        except Exception:
            assert False, "Exception handler caught cancellation"
    except asyncio.CancelledError as signal:
        assert str(signal) == "FIRST"
        with anyio.CancelScope(shield=True):
            await exchange("cleanup")
        return json.dumps("suppressed")
    assert False, "cancel not delivered"


asyncio.run(main())
