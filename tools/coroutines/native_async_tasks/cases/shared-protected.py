import asyncio
import json


async def child():
    return await exchange("S")


async def fail(label):
    await exchange(label)
    raise ValueError(label)


async def owner(label, shared):
    try:
        pass
    finally:
        try:
            await asyncio.gather(fail(label), shared)
        except ValueError:
            await exchange(label + "-release")


async def main():
    shared = asyncio.create_task(child())
    a = asyncio.create_task(owner("A", shared))
    b = asyncio.create_task(owner("B", shared))
    await asyncio.gather(a, b)
    return json.dumps("ok")


asyncio.run(main())
