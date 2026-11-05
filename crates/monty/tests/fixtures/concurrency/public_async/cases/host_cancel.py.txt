import asyncio
import json


async def main():
    trace = []
    try:
        await exchange('body')
    finally:
        trace.append(await exchange('release'))
        await exchange('audit:' + json.dumps(trace))
    return json.dumps('ordinary-success-is-forbidden')


asyncio.run(main())
