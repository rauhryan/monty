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


async def other():
    await exchange('C')
    raise TypeError('inner-error')


async def main():
    try:
        async with asyncio.TaskGroup() as outer:
            outer.create_task(fail())
            async with asyncio.TaskGroup() as _middle:
                async with asyncio.TaskGroup() as inner:
                    b = inner.create_task(child())
                    inner.create_task(other())
    except RuntimeError as error:
        assert len(error.args[0]) == 2
        assert str(error.args[0][0]) == 'outer-error'
        nested = error.args[0][1]
        assert len(nested.args[0]) == 1
        inner_error = nested.args[0][0]
        assert len(inner_error.args[0]) == 1
        assert str(inner_error.args[0][0]) == 'inner-error'
        assert b.cancelled()
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
