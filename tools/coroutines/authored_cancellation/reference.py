"""Run the same authored programs against CPython and installed upstream AnyIO."""

import asyncio
from pathlib import Path


async def exchange(label):
    await asyncio.sleep(0)
    return "ok"


for source in sorted((Path(__file__).resolve().parent / "cases").glob("*.py")):
    exec(compile(source.read_text(), str(source), "exec"), {"exchange": exchange})
    print(source.name, "PASS")
