import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    token = []
    token.append(token)
    token.append(q)
    task = asyncio.create_task(q.put(token))

    async def cancel():
        assert task.cancel()
        await exchange("cancelled-reservation")

    cancel_task = asyncio.create_task(cancel())
    await exchange("control")
    try:
        await task
        assert False
    except ValueError as error:
        assert str(error) == "guest"
    assert task.cancelled()
    assert q.empty()
    await cancel_task
    await q.put("next")
    assert await q.get() == "next" and q.empty()
    await exchange("verified")
    return json.dumps("ok")


asyncio.run(main())
