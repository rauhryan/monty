import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    async def producer():
        await exchange("release")
        await q.put("cleanup")

    try:
        await exchange("body")
    finally:
        values = await asyncio.gather(q.get(), producer())
        assert values == ["cleanup", None]
        await exchange("parent-release")
    return json.dumps("ok")


asyncio.run(main())
