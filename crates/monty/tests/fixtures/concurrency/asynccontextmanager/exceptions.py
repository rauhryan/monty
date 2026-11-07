# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    body = ValueError('body')
    replacement = RuntimeError('cleanup')
    seen = []

    @asynccontextmanager
    async def manager(suppress):
        try:
            yield
        except ValueError as error:
            assert error is body
            assert await exchange('suppressed' if suppress else 'replaced') == 'ok'
            seen.append(error)
            if not suppress:
                raise replacement from error

    async with manager(True):
        raise body
    try:
        async with manager(False):
            raise body
    except RuntimeError as error:
        assert error is replacement
        assert error.__cause__ is body
        assert error.__context__ is body
    else:
        assert False
    assert seen == [body, body]
    return json.dumps('ok')


asyncio.run(main())
