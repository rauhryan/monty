import asyncio
import json


async def child(label):
    token = [label + '-error']
    token.append(token)
    failure = ValueError(token)
    token.append(failure)
    try:
        await exchange(label)
        raise failure
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
        assert [e.args[0][0] for e in errors] == ['A-error', 'B-error']
        assert errors[0].args[0][1] is errors[0].args[0]
        assert errors[1].args[0][1] is errors[1].args[0]
        assert errors[0].args[0][2] is errors[0]
        assert errors[1].args[0][2] is errors[1]
        await exchange('parent-release')
    return json.dumps('ok')


asyncio.run(main())
