import asyncio
import json


async def main():
    first = await exchange('A')
    second = await exchange('B')
    return json.dumps([first, second])


asyncio.run(main())
