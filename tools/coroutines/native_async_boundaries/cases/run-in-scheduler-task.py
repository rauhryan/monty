import asyncio
import json


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')


async def main():
    asyncio.create_task(child())
    await exchange('control')
    return 'inner'


def wrapper():
    return asyncio.run(main())


async def worker():
    return wrapper()


async def sibling():
    await exchange('outside')
    return 'outer'


values = await asyncio.gather(worker(), sibling())
assert values == ['inner', 'outer']
json.dumps('ok')
