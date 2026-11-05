import asyncio
import json


async def main():
    queue = asyncio.Queue(maxsize=1)
    trace = []

    async def producer():
        await queue.put('one')
        await queue.put('two')
        trace.append(await exchange('producer-done'))

    async def consumer():
        await exchange('allow-get')
        values = [await queue.get(), await queue.get()]
        return values

    values = await asyncio.gather(producer(), consumer())
    return json.dumps([values[1], trace])


asyncio.run(main())
