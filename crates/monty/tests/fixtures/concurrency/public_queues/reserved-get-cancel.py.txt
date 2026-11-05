import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    token = []
    token.append(token)
    token.append(q)
    await q.put(token)
    task = asyncio.create_task(q.get())

    async def cancel():
        assert task.cancel()
        await exchange("cancelled-reservation")

    cancel_task = asyncio.create_task(cancel())
    await exchange("control")
    try:
        await task
        assert False
    except asyncio.CancelledError as error:
        assert str(error) == "guest"
    assert task.cancelled()
    assert q.qsize() == 1 and await q.get() is token and q.empty()
    await cancel_task
    await q.put("next")
    assert await q.get() == "next" and q.empty()
    await exchange("verified")
    return json.dumps("ok")


asyncio.run(main())
