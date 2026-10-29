import asyncio
import json


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')


async def main(fail):
    token = []
    token.append(token)
    error = ValueError(token)
    token.append(error)
    asyncio.create_task(child())
    await exchange('control')
    if fail:
        raise error
    return token


def wrapper(fail):
    try:
        value = asyncio.run(main(fail))
        assert value[0] is value and value[1].args[0] is value
    except ValueError as error:
        assert fail
        value = error.args[0]
        assert value[0] is value and value[1] is error
    return json.dumps('ok')


wrapper(False)
