import asyncio


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
        async with asyncio.TaskGroup() as group:
            group.create_task(first())
            group.create_task(second())
            await exchange("body")
            assert False
    finally:
        await exchange("parent-release")


asyncio.run(main())
