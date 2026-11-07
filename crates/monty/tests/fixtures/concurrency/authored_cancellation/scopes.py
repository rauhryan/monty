import asyncio
import anyio
import json


async def child():
    with anyio.CancelScope() as scope:
        scope.cancel()
        raise asyncio.CancelledError('foreign')


async def main():
    task = asyncio.create_task(child())
    await exchange('before')
    try:
        await task
    except asyncio.CancelledError as error:
        assert str(error) == 'foreign'
    else:
        assert False, 'scope suppressed unrelated cancellation'
    assert task.cancelled()
    await exchange('completed')
    assert task.cancelled()
    return json.dumps('ok')


asyncio.run(main())
