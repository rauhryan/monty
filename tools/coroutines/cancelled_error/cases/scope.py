# ruff: noqa: F821
import asyncio
import anyio
import json


async def main():
    with anyio.CancelScope() as scope:
        scope.cancel()
        try:
            await exchange("must-not-run")
        except ValueError:
            assert False
        except Exception:
            assert False
        except asyncio.CancelledError:
            with anyio.CancelScope(shield=True):
                await exchange("cleanup")
            await exchange("must-not-run-again")
    assert scope.cancelled_caught
    try:
        with anyio.CancelScope():
            raise asyncio.CancelledError("user")
    except asyncio.CancelledError as signal:
        assert str(signal) == "user"
    else:
        assert False, "foreign cancellation suppressed"
    try:
        raise ValueError("validation")
    except asyncio.CancelledError:
        assert False
    except ValueError:
        pass
    await exchange("after")
    return json.dumps("ok")


asyncio.run(main())
