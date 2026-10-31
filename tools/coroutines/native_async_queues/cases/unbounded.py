import asyncio
import json


async def main():
    q = asyncio.Queue()
    for i in range(40):
        await q.put(i)
    assert q.qsize() == 40 and not q.full()
    await exchange("stored")
    for i in range(40):
        assert await q.get() == i
    assert q.empty()
    return json.dumps("ok")


asyncio.run(main())
