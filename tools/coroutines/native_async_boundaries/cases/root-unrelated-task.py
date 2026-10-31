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
    return json.dumps('ok')


async def outside_child():
    try:
        await exchange('outside')
    finally:
        await exchange('outside-release')


def wrapper():
    return asyncio.run(main())


outside = asyncio.create_task(outside_child())
value = wrapper()
assert not outside.done()
await exchange('after-run')
await outside
value
