import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    token = []
    token.append(token)
    token.append(q)
    await q.put(token)
    task = asyncio.create_task(q.put(token))
    await exchange("blocked")
    assert task.cancel()
    try:
        await task
        assert False
    except ValueError:
        pass
    assert q.qsize() == 1 and await q.get() is token
    assert q.empty()
    await exchange("delivered")
    return json.dumps("ok")


asyncio.run(main())
