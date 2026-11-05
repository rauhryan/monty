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


class Resource:
    async def __aenter__(self):
        return self
    async def __aexit__(self,t,e,tb):
        await exchange('parent-release')

async def main():
    async with Resource():
        try:
            await asyncio.gather(left(),right())
        except ValueError:
            pass
    return json.dumps('ok')

asyncio.run(main())
