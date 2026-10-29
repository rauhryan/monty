import asyncio
import json


async def main():
    for size in [0, -2, True, False, 1, 3, 4294967295]:
        q = asyncio.Queue(maxsize=size)
        assert q.maxsize == size and q.empty() and q.qsize() == 0 and not q.full()
        token = []
        token.append(token)
        put = q.put(item=token)
        get = q.get()
        assert q.empty()
        await exchange("before-" + str(size))
        await put
        assert q.qsize() == 1 and not q.empty()
        assert q.full() == (size == 1)
        value = await get
        assert value is token and value[0] is token
        assert q.empty() and not q.full()
    return json.dumps("ok")


asyncio.run(main())
