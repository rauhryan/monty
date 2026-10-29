import asyncio
import json


async def left():
    await exchange('A')
    raise ValueError('left-error')


ready = False


async def right():
    try:
        while not ready:
            await exchange('B')
    finally:
        await exchange('child-release')


async def main():
    global ready
    try:
        await asyncio.gather(left(), right())
    except ValueError:
        pass
    finally:
        ready = True
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
