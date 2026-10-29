import asyncio


async def child(continuation):
    return continuation._native1()


async def direct_gather():
    return await asyncio.gather(exchange('direct-A'), asyncio.gather(exchange('direct-B')))
