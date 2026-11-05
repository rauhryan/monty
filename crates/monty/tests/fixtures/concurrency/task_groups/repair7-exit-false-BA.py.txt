import asyncio
import json


class Resource:
    def __init__(self, label):
        self.label = label

    async def __aenter__(self):
        return self

    async def __aexit__(self, typ, error, tb):
        await exchange(self.label + '-release')
        return False


async def child(label):
    async with Resource(label):
        await exchange(label)
        raise ValueError(label + '-error')


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
