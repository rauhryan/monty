import asyncio
import json


async def child():
    token = []
    token.append(token)
    await exchange("A")
    raise ValueError(token)


async def main():
    task = asyncio.create_task(child())
    first = None
    for _ in range(2):
        try:
            await task
            assert False
        except ValueError as error:
            assert error.args[0][0] is error.args[0]
            if first is None:
                first = error
            assert error is first and task.exception() is first
    for _ in range(2):
        try:
            task.result()
            assert False
        except ValueError as error:
            assert error is first
    assert task.done() and not task.cancelled() and not task.cancel()
    await exchange("inspect")
    assert task.exception() is first
    return json.dumps("ok")


asyncio.run(main())
