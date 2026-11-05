import asyncio
import json


async def child():
    try:
        await exchange("body")
    finally:
        await exchange("child-release")


async def main():
    asyncio.create_task(child())
    await exchange("control")
    raise ValueError("root-error")


try:
    asyncio.run(main())
except ValueError as error:
    assert str(error) == "root-error"
    outcome = "ok"
json.dumps(outcome)
