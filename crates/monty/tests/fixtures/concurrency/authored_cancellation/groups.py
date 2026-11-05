import asyncio
import anyio
import json


async def cancelled():
    raise asyncio.CancelledError('authored')


async def sibling(queue, seen):
    seen.append(await queue.get())


async def main():
    for kind in ['asyncio', 'anyio']:
        queue = asyncio.Queue(1)
        seen = []
        if kind == 'asyncio':
            async with asyncio.TaskGroup() as group:
                task = group.create_task(cancelled())
                group.create_task(sibling(queue, seen))
                await exchange(kind)
                await queue.put('delivered')
            assert task.cancelled()
        else:
            async with anyio.create_task_group() as group:
                group.start_soon(cancelled)
                group.start_soon(sibling, queue, seen)
                await exchange(kind)
                await queue.put('delivered')
        assert seen == ['delivered']
    await exchange('drained')
    return json.dumps('ok')


asyncio.run(main())
