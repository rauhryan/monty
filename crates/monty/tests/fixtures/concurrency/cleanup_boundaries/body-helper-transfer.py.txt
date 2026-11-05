import asyncio
import json


async def left():
    await exchange('A')
    raise ValueError('left-error')


async def right():
    try:
        await exchange('B')
    finally:
        await exchange('child-release')


async def joined():
    try:
        await asyncio.gather(left(), right())
    except ValueError:
        pass


async def helper():
    return await joined()


async def main():
    try:
        await helper()
    finally:
        await release()
    return json.dumps('ok')


async def release():
    await exchange('parent-release')


asyncio.run(main())
