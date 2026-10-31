import asyncio
import json


async def main():
    trace = []
    try:
        try:
            trace.append(await exchange('body'))
            raise ValueError('body-error')
        finally:
            trace.append(await exchange('release'))
    except ValueError as error:
        trace.append(str(error))
    return json.dumps(trace)


asyncio.run(main())
