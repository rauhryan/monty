import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    a = asyncio.create_task(q.get())
    b = asyncio.create_task(q.get())
    await exchange("blocked")
    assert a.cancel()
    assert a.cancel()
    try:
        await a
        assert False
    except asyncio.CancelledError as error:
        assert str(error) == "guest"
    assert a.cancelled() and not a.cancel()
    await q.put("B")
    assert await b == "B"
    assert q.empty()
    await exchange("done")
    return json.dumps("ok")


asyncio.run(main())
