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
    return json.dumps('ok')


events = []


def inner():
    try:
        return asyncio.run(main())
    finally:
        events.append('inner')


def middle():
    try:
        return inner()
    finally:
        events.append('middle')


def wrapper():
    try:
        return middle()
    finally:
        events.append('outer')


value = wrapper()
assert events == ['inner', 'middle', 'outer']
value
