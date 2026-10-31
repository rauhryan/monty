import asyncio
import json


async def main():
    q = asyncio.Queue(1)
    token = []
    token.append(token)
    token.append(q)

    async def witness(label):
        await exchange(label)

    observer = asyncio.create_task(witness("put-reserved"))
    await q.put(token)
    await observer
    observer = asyncio.create_task(witness("get-reserved"))
    value = await q.get()
    assert value is token and value[0] is token and value[1] is q
    await observer
    assert q.empty()
    return json.dumps("ok")


asyncio.run(main())
