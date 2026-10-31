import asyncio
import json


async def left():
    await exchange("A")
    raise ValueError("left-error")


async def right():
    try:
        await exchange("B")
    finally:
        await exchange("child-release")


async def main():
    try:
        await exchange("body")
    finally:
        try:
            first = asyncio.create_task(left())
            second = asyncio.create_task(right())
            await asyncio.gather(first, second)
        except ValueError as error:
            assert str(error) == "left-error"
        finally:
            await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
