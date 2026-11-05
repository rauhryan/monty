import asyncio


async def child(label):
    try:
        raise ValueError(label + '-error')
    finally:
        await exchange(label + '-release')


async def main():
    try:
        async with asyncio.TaskGroup() as group:
            group.create_task(child('A'))
            group.create_task(child('B'))
            await exchange('body')
            assert False
    finally:
        await exchange('parent-release')


asyncio.run(main())
