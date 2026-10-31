import asyncio
import json


async def helper(label):
    raise ValueError(label + '-error')


async def child(label):
    try:
        await exchange(label)
        await helper(label)
    finally:
        await exchange(label + '-release')


async def main():
    try:
        async with asyncio.TaskGroup() as group:
            a = group.create_task(child('A'))
            b = group.create_task(child('B'))
    except RuntimeError as error:
        errors = error.args[0]
        assert not a.cancelled() and not b.cancelled()
        assert len(errors) == 2
        assert errors[0] is a.exception()
        assert errors[1] is b.exception()
        assert [str(e) for e in errors] == ['A-error', 'B-error']
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
