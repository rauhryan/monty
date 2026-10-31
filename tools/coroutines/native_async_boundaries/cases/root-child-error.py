import asyncio
import json

saved = TypeError('child-error')
tasks = []


async def child():
    try:
        await exchange('body')
    finally:
        await exchange('child-release')
        raise saved


async def main():
    tasks.append(asyncio.create_task(child()))
    await exchange('control')
    return json.dumps('ok')


def wrapper():
    return asyncio.run(main())


value = wrapper()
assert tasks[0].exception() is saved
assert not tasks[0].cancelled()
value
