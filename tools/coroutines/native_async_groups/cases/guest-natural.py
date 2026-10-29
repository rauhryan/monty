import asyncio
import json


async def child():
    token = ['cleanup-error']
    token.append(token)
    try:
        await exchange('A')
    finally:
        await exchange('A-release')
        raise ValueError(token)


async def main():
    task = asyncio.create_task(child())
    await exchange('control')
    assert task.cancel()
    await exchange('repeat')
    assert task.cancel()
    try:
        await task
    except ValueError as error:
        assert error is task.exception()
        assert not task.cancelled()
        assert error.args[0][0] == 'cleanup-error'
        assert error.args[0][1] is error.args[0]
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
