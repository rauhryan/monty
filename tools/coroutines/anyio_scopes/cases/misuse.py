import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def foreign(scope):
    try:
        scope.__exit__(None, None, None)
        assert False
    except RuntimeError:
        pass
    try:
        scope.__enter__()
        assert False
    except RuntimeError:
        pass
async def main():
    a, b = anyio.CancelScope(), anyio.CancelScope()
    assert a.__enter__() is a
    assert b.__enter__() is b
    try:
        a.__exit__(None, None, None)
        assert False
    except RuntimeError:
        pass
    await asyncio.create_task(foreign(b))
    assert b.__exit__(None, None, None) is False
    assert a.__exit__(None, None, None) is False
    try:
        b.__exit__(None, None, None)
        assert False
    except RuntimeError:
        pass
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
