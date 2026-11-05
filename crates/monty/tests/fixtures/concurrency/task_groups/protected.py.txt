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


async def main():
    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(first())
            group.create_task(second())
            try:
                await exchange("body")
            finally:
                values = await asyncio.gather(exchange("C"), exchange("D"))
                assert values == ["c", "d"]
            await exchange("must-not-run")
    except Exception as error:
        try:
            errors = error.exceptions
        except AttributeError:
            errors = error.args[0]
        assert len(errors) == 1 and str(errors[0]) == "first"
    await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
