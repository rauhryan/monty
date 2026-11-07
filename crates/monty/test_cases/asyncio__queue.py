import asyncio


async def main():
    for size in [0, -1, 1, 3]:
        queue = asyncio.Queue(maxsize=size)
        assert queue.maxsize == size
        assert queue.empty() and queue.qsize() == 0 and not queue.full()
        token = []
        token.append(token)
        put = queue.put(item=token)
        get = queue.get()
        assert queue.empty()
        await put
        assert queue.qsize() == 1 and queue.full() == (size == 1)
        value = await get
        assert value is token and value[0] is token and queue.empty()
    queue = asyncio.Queue(1)
    async with asyncio.TaskGroup() as group:
        a = group.create_task(queue.put('one'))
        b = group.create_task(queue.put('two'))
        assert [await queue.get(), await queue.get()] == ['one', 'two']
    assert a.result() is None and b.result() is None
    assert queue.empty()
    assert await asyncio.gather(queue.put(3), queue.get()) == [None, 3]


asyncio.run(main())
