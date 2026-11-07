import asyncio
import json


async def fail():
    await exchange('A')
    raise ValueError('outer-error')


async def child():
    try:
        await exchange('B')
        await exchange('must-not-run')
    finally:
        await exchange('B-release')


async def main():
    try:
        async with asyncio.TaskGroup() as outer:
            outer.create_task(fail())
            async with asyncio.TaskGroup() as _middle:
                async with asyncio.TaskGroup() as inner:
                    b = inner.create_task(child())
                    await exchange('body')
    except RuntimeError as error:
        assert len(error.args[0]) == 1
        assert str(error.args[0][0]) == 'outer-error'
        assert b.cancelled()
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
