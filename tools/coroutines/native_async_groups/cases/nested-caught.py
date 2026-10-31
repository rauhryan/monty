import asyncio
import json


async def outer_child():
    return await exchange("O")


async def fail():
    await exchange("A")
    raise ValueError("inner-error")


async def sibling():
    try:
        await exchange("B")
    finally:
        await exchange("child-release")


async def main():
    async with asyncio.TaskGroup() as outer:
        surviving = outer.create_task(outer_child())
        try:
            async with asyncio.TaskGroup() as inner:
                inner.create_task(fail())
                inner.create_task(sibling())
                await exchange("body")
        except Exception as error:
            try:
                errors = error.exceptions
            except AttributeError:
                errors = error.args[0]
            assert len(errors) == 1 and str(errors[0]) == "inner-error"
        assert not surviving.done()
        await exchange("after-inner")
    assert surviving.result() == "o"
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
