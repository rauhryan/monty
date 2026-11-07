# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    closed = []

    @asynccontextmanager
    async def empty():
        assert await exchange('empty') == 'ok'
        if False:
            yield

    try:
        async with empty():
            assert False
    except RuntimeError as error:
        assert str(error) == "generator didn't yield"
    else:
        assert False

    @asynccontextmanager
    async def twice(throwing):
        try:
            try:
                yield
            except ValueError:
                assert throwing
            assert await exchange('throw-second' if throwing else 'normal-second') == 'ok'
            yield
        finally:
            assert await exchange('throw-closed' if throwing else 'normal-closed') == 'ok'
            closed.append(True)

    for throwing in [False, True]:
        try:
            async with twice(throwing):
                if throwing:
                    raise ValueError('body')
        except RuntimeError as error:
            expected = "generator didn't stop after athrow()" if throwing else "generator didn't stop"
            assert str(error) == expected
        else:
            assert False
    assert closed == [True, True]
    return json.dumps('ok')


asyncio.run(main())
