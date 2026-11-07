import asyncio
from contextlib import asynccontextmanager


events = []


async def step(value):
    events.append(value)
    return value


@asynccontextmanager
async def manager(value=1):
    await step('enter')
    try:
        yield value
    finally:
        await step('exit')


@manager(7)
async def decorated(value):
    return await step(value)


@asynccontextmanager
async def broken():
    raise StopIteration('inner')
    yield


async def main():
    async with manager(value=3) as value:
        assert value == 3
    assert await decorated(8) == 8
    assert await decorated(9) == 9
    assert events == ['enter', 'exit', 'enter', 8, 'exit', 'enter', 9, 'exit']
    try:
        async with broken():
            assert False
    except RuntimeError as error:
        assert str(error) == 'async generator raised StopIteration'
        assert isinstance(error.__cause__, StopIteration)
        assert str(error.__cause__) == 'inner'
    else:
        assert False
    original = StopIteration('body')
    try:
        async with manager():
            raise original
    except StopIteration as error:
        assert error is original
    else:
        assert False


asyncio.run(main())
