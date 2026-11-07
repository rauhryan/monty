# ruff: noqa: F821
import asyncio
import json


async def worker(queue):
    assert await queue.get() == 'job'
    assert queue.empty()
    await exchange('work')
    queue.task_done()


async def main():
    queue = asyncio.Queue(1)
    await queue.join()
    await queue.put('job')
    task = asyncio.create_task(worker(queue))
    first = asyncio.create_task(queue.join())
    cancelled = asyncio.create_task(queue.join())
    await exchange('control')
    assert not first.done()
    cancelled.cancel()
    try:
        await cancelled
    except asyncio.CancelledError:
        pass
    else:
        assert False
    await first
    await task
    await queue.join()
    try:
        queue.task_done()
    except ValueError as error:
        assert str(error) == 'task_done() called too many times'
    else:
        assert False
    await queue.put('second')
    assert await queue.get() == 'second'
    queue.task_done()
    await queue.join()
    await queue.put('third')
    blocked = asyncio.create_task(queue.put('cancelled-put'))
    await exchange('put-control')
    blocked.cancel()
    try:
        await blocked
    except asyncio.CancelledError:
        pass
    else:
        assert False
    queue.task_done()
    await queue.join()
    assert await queue.get() == 'third'
    assert queue.empty()
    return json.dumps('ok')


asyncio.run(main())
