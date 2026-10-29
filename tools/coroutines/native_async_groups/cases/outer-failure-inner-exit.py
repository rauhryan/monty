import asyncio
import json
async def fail():
    await exchange('A')
    raise ValueError('outer-error')
async def child():
    try:
        await exchange('B')
        await exchange('must-not-run')
    finally:
        await exchange('B-release')
async def main():
    try:
        async with asyncio.TaskGroup() as outer:
            outer.create_task(fail())
            async with asyncio.TaskGroup() as inner:
                inner.create_task(child())
    except RuntimeError:
        pass
    return json.dumps('ok')
asyncio.run(main())
