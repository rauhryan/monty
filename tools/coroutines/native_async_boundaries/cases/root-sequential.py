import asyncio
import json


async def child(n):
    try:
        await exchange('body' + n)
    finally:
        await exchange('release' + n)


async def main(n):
    asyncio.create_task(child(n))
    await exchange('control' + n)
    return n


def wrapper(n):
    return asyncio.run(main(n))


assert wrapper('1') == '1'
assert wrapper('2') == '2'
json.dumps('ok')
