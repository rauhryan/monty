import asyncio
import json


async def first():
    await exchange("A")
    raise ValueError("first")


async def second():
    try:
        await exchange("B")
    finally:
        await exchange("child-release")
        raise TypeError("second")


async def main():
    try:
        try:
            async with asyncio.TaskGroup() as group:
                group.create_task(first())
                group.create_task(second())
                await exchange("body")
                assert False
        except Exception as error:
            try:
                errors = error.exceptions
            except AttributeError:
                errors = error.args[0]
            assert len(errors) == 2
            assert str(errors[0]) == "first" and str(errors[1]) == "second"
    finally:
        await exchange("parent-release")
    async with asyncio.TaskGroup() as group:
        pass
    return json.dumps("ok")


asyncio.run(main())
