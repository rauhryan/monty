# ruff: noqa: F821
import asyncio
import json
from contextlib import asynccontextmanager


async def main():
    token = []
    token.append(token)
    events = []

    @asynccontextmanager
    async def manager(label):
        assert await exchange(label + '-enter') == 'ok'
        events.append(label + '-enter')
        try:
            yield token
        finally:
            assert await exchange(label + '-exit') == 'ok'
            events.append(label + '-exit')

    for label in ['a', 'b']:
        async with manager(label) as value:
            assert value is token
            assert await exchange(label + '-body') == 'ok'
            events.append(label + '-body')
    assert events == ['a-enter', 'a-body', 'a-exit', 'b-enter', 'b-body', 'b-exit']
    return json.dumps('ok')


asyncio.run(main())
