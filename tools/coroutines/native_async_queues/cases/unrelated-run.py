import asyncio
import json

queue = asyncio.Queue()
outside = asyncio.create_task(queue.get())


async def main():
    own = asyncio.create_task(queue.get())
    await exchange("control")
    assert not own.done() and not outside.done()
    return "inner"


async def finish():
    assert not outside.done()
    await queue.put("outer")
    assert await outside == "outer"
    assert queue.empty()
    await exchange("after-run")
    return json.dumps("ok")


assert asyncio.run(main()) == "inner"
assert not outside.done()
asyncio.run(finish())
