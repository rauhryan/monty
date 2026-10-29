import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    async def producer():
        await exchange("release")
        await q.put("cleanup")

    async def worker():
        try:
            await exchange("body")
        finally:
            values = await asyncio.gather(q.get(), producer())
            assert values == ["cleanup", None]

    task = asyncio.create_task(worker())
    await exchange("control1")
    assert task.cancel()
    await exchange("control2")
    assert task.cancel()
    try:
        await task
        assert False
    except ValueError as error:
        assert str(error) == "guest"
    assert task.cancelled() and q.empty()
    return json.dumps("ok")


asyncio.run(main())
