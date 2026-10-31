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


async def fail():
    raise RuntimeError('later-error')


async def done():
    return None


async def orphaner():
    try:
        await asyncio.gather(left(), right())
    except ValueError:
        return None


async def work():
    try:
        await asyncio.gather(orphaner())
    finally:
        await exchange('parent-release')


async def main():
    try:
        await work()
        return json.dumps('ok')
    except TypeError:
        raise


asyncio.run(main())
