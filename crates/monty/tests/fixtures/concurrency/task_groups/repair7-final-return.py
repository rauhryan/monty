import asyncio
import json


class Resource:
    async def __aenter__(self):
        return self

    async def __aexit__(self, typ, error, tb):
        await exchange('release')
        return True


async def child():
    try:
        raise ValueError('replaced-by-return')
    finally:
        await exchange('release')
        return 'discarded'


async def main():
    task = asyncio.create_task(child())
    await exchange('control')
    assert task.cancel()
    await exchange('repeat')
    assert task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        assert task.cancelled()
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
