import asyncio
import json


async def main():
    q = asyncio.Queue(1)

    async def blocked():
        try:
            await q.get()
        finally:
            await exchange("child-release")

    async def bad():
        await exchange("fail")
        raise TypeError("failure")

    try:
        try:
            await asyncio.gather(bad(), blocked())
        finally:
            await exchange("parent-release")
    except TypeError:
        pass
    assert q.empty()
    return json.dumps("ok")


asyncio.run(main())
