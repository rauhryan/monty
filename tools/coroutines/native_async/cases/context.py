import asyncio
import json

trace = []


class Resource:
    async def __aenter__(self):
        trace.append(await exchange('acquire'))
        return self

    async def __aexit__(self, kind, value, traceback):
        trace.append(await exchange('release'))


async def main():
    async with Resource():
        trace.append(await exchange('body'))
    return json.dumps(trace)


asyncio.run(main())
