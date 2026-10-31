import asyncio
import json


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')


async def main():
    asyncio.create_task(child())
    await exchange('control')
    raise ValueError('root-error')


def wrapper():
    return asyncio.run(main())


try:
    wrapper()
except ValueError as error:
    if str(error) == 'FIRST':
        raise
    assert str(error) == 'root-error'
    result = json.dumps('caught')
result
