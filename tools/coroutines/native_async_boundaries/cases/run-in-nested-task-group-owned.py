import asyncio
import json


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')


async def grouped():
    async with asyncio.TaskGroup() as group:
        group.create_task(child())
        await exchange('control')
    return 'inner'


async def main():
    return await asyncio.create_task(grouped())


def wrapper():
    return asyncio.run(main())


async def worker():
    return wrapper()


async def sibling():
    await exchange('outside')
    return 'outer'


async def entry():
    values = await asyncio.gather(worker(), sibling())
    assert values == ['inner', 'outer']
    return json.dumps('ok')


await entry()
