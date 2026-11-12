# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    events = []

    @asynccontextmanager
    async def manager(label):
        assert await exchange(label + '-enter') == 'ok'
        events.append(label + '-enter')
        try:
            yield label
        finally:
            assert await exchange(label + '-exit') == 'ok'
            events.append(label + '-exit')

    async with manager('outer') as outer, manager('inner') as inner:
        assert outer == 'outer' and inner == 'inner'
        assert await exchange('body') == 'ok'
    assert events == ['outer-enter', 'inner-enter', 'inner-exit', 'outer-exit']
    return json.dumps('ok')


asyncio.run(main())
