import asyncio
import json


async def child():
    try:
        await exchange('B')
        await exchange('must-not-run')
    finally:
        await exchange('B-release')


async def owner():
    try:
        async with asyncio.TaskGroup() as _outer:
            async with asyncio.TaskGroup() as inner:
                inner.create_task(child())
    finally:
        await exchange('owner-release')


async def main():
    task = asyncio.create_task(owner())
    await exchange('control')
    assert task.cancel()
    await exchange('repeat')
    assert task.cancel()
    try:
        await task
    except asyncio.CancelledError as error:
        assert str(error) == 'guest'
        assert task.cancelled()
    return json.dumps('ok')


asyncio.run(main())
