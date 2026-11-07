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


async def recovery():
    try:
        await exchange('recovery')
    finally:
        await exchange('helper-release')


async def main():
    try:
        await asyncio.gather(left(), right())
    except ValueError:
        await recovery()
    finally:
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
