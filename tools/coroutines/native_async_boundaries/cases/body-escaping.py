import asyncio
import json

saved = ValueError('left-error')


async def left():
    await exchange('A')
    raise saved


async def right():
    try:
        await exchange('B')
    finally:
        await exchange('child-release')


async def main():
    try:
        await asyncio.gather(left(), right())
    finally:
        await exchange('parent-release')
    return json.dumps('ok')


try:
    asyncio.run(main())
except ValueError as error:
    assert error is saved
    result = json.dumps('ok')
result
