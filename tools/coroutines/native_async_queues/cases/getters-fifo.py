import asyncio
import json


async def main():
    q = asyncio.Queue()
    a = asyncio.create_task(q.get())
    b = asyncio.create_task(q.get())
    c = asyncio.create_task(q.get())
    await exchange("empty")
    assert not a.done() and not b.done() and not c.done()
    await asyncio.gather(q.put("A"), q.put("B"), q.put("C"))
    assert await asyncio.gather(a, b, c) == ["A", "B", "C"]
    assert q.empty()
    await exchange("done")
    return json.dumps("ok")


asyncio.run(main())
