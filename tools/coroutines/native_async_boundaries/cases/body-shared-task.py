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


async def main():
    task = asyncio.create_task(right())
    try:
        await asyncio.gather(left(), task, task)
    except ValueError:
        pass
    finally:
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
