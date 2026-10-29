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


try:
    wrapper()
except ValueError as error:
    assert str(error) == 'root-error'
    result = json.dumps('caught')
assert events == ['inner', 'middle', 'outer']
result
