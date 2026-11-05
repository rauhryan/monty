import asyncio


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
    await owner()


asyncio.run(main())
