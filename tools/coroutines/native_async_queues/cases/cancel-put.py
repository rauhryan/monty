import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    await q.put("seed")

    a = asyncio.create_task(q.put("A"))
    b = asyncio.create_task(q.put("B"))
    await exchange("blocked")
    assert a.cancel()
    assert a.cancel()
    try:
        await a
        assert False
    except ValueError as error:
        assert str(error) == "guest"
    assert a.cancelled() and not a.cancel()
    assert await q.get() == "seed"
    assert await b == None
    assert await q.get() == "B" and q.empty()
    await exchange("done")
    return json.dumps("ok")


asyncio.run(main())
