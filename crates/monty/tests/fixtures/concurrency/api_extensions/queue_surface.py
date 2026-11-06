import asyncio
import json


async def main():
    for size in ['1', None, [], 4294967296]:
        try:
            asyncio.Queue(size)
            assert False
        except (TypeError, ValueError):
            pass
    q = asyncio.Queue()
    try:
        await q
        assert False
    except TypeError:
        pass
    for name in ['put_nowait', 'get_nowait', 'shutdown']:
        try:
            getattr(q, name)
            assert False
        except AttributeError:
            pass
    put = q.put('one')
    await put
    try:
        await put
        assert False
    except RuntimeError:
        pass
    get = q.get()
    assert await get == 'one'
    try:
        await get
        assert False
    except RuntimeError:
        pass
    assert q.empty()
    q.task_done()
    await q.join()
    await exchange('checked')
    return json.dumps('ok')


asyncio.run(main())
