import asyncio
import json

trace = []


async def child(label):
    value = await exchange(label)
    trace.append(label)
    return value


async def main():
    values = await asyncio.gather(child('A'), child('B'))
    return json.dumps({'values': values, 'trace': trace})


asyncio.run(main())
