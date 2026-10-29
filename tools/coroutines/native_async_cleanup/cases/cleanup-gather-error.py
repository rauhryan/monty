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
    try:
        await exchange('body')
    finally:
        try:
            await asyncio.gather(left(), right())
        finally:
            await exchange('parent-release')
asyncio.run(main())
