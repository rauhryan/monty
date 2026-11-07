# ruff: noqa: F821
import asyncio
import json


async def fail(error):
    raise error


async def main():
    error = ValueError('collected')
    task = asyncio.create_task(fail(error))
    # Duplicate references must receive the identical exception object.
    group = asyncio.gather(task, exchange('success'), task, return_exceptions=True)
    values = await group
    assert values[0] is error and values[2] is error
    assert values[1] == 'ok'
    assert await group is values
    assert await asyncio.gather(return_exceptions=True) == []
    nested = await asyncio.gather(asyncio.gather(fail(error)), return_exceptions=True)
    assert nested[0] is error
    # Already completed failures and reused coroutines take the synchronous commit path.
    values = await asyncio.gather(task, return_exceptions=True)
    assert values[0] is error
    coro = fail(error)
    values = await asyncio.gather(coro, return_exceptions=True)
    assert values[0] is error
    values = await asyncio.gather(
        asyncio.gather(coro), exchange('later'), return_exceptions=True
    )
    assert isinstance(values[0], RuntimeError) and values[1] == 'ok'
    repeated = await asyncio.gather(coro, coro, return_exceptions=True)
    assert isinstance(repeated[0], RuntimeError) and repeated[0] is repeated[1]
    cancelled = await asyncio.gather(
        fail(asyncio.CancelledError()), return_exceptions=True
    )
    assert isinstance(cancelled[0], asyncio.CancelledError)
    # Collected failures must not reappear in a TaskGroup's lifetime error list.
    async with asyncio.TaskGroup():
        assert (await asyncio.gather(fail(error), return_exceptions=True))[0] is error
        nested = asyncio.gather(fail(error))
        assert (await asyncio.gather(nested, return_exceptions=True))[0] is error
    return json.dumps('ok')


asyncio.run(main())
