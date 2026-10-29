import asyncio
import json


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')


async def main():
    asyncio.create_task(child())
    try:
        wrapper()
    except RuntimeError as error:
        assert str(error) == 'asyncio.run() cannot be called from a running event loop'
    await exchange('control')
    return json.dumps('ok')


def wrapper():
    return asyncio.run(main())


wrapper()
