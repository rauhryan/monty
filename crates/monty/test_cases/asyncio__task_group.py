import asyncio


async def value(token):
    return token


async def nested(token):
    async with asyncio.TaskGroup() as group:
        task = group.create_task(value(token))
    return task.result()


async def main():
    token = []
    async with asyncio.TaskGroup() as outer:
        first = outer.create_task(value(token))
        second = outer.create_task(nested(token))
        async with asyncio.TaskGroup() as inner:
            third = inner.create_task(value(token))
        assert third.result() is token
    assert first.result() is token and second.result() is token
    assert await first is token
    assert all(item is token for item in await asyncio.gather(first, second, first))
    try:
        async with asyncio.TaskGroup():
            raise ValueError('body-error')
    except Exception as aggregate:
        try:
            errors = aggregate.exceptions
        except AttributeError:
            errors = aggregate.args[0]
        assert len(errors) == 1
        assert isinstance(errors[0], ValueError)
        assert str(errors[0]) == 'body-error'
    else:
        assert False


asyncio.run(main())
