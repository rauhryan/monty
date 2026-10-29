"""CPython reference for the Queue cases whose semantics match the standard API."""

import ast
import asyncio
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


async def execute(name, case):
    pending = {}
    calls = []
    position = 0
    loop = asyncio.get_running_loop()

    def deliver():
        nonlocal position
        if position == len(case["batches"]):
            return
        batch = case["batches"][position]
        if all(label in pending for label in batch):
            position += 1
            for label, value in batch.items():
                pending.pop(label).set_result(value)
            loop.call_soon(deliver)

    async def exchange(label):
        assert label not in pending
        calls.append(label)
        future = loop.create_future()
        pending[label] = future
        loop.call_soon(deliver)
        try:
            return await future
        finally:
            if future.cancelled():
                pending.pop(label, None)

    path = ROOT / "cases" / (name + ".py")
    module = ast.parse(path.read_text(), filename=str(path))
    assert ast.unparse(module.body[-1]) == "asyncio.run(main())"
    module.body.pop()
    namespace = {"exchange": exchange}
    exec(compile(module, str(path), "exec"), namespace)
    result = await asyncio.wait_for(namespace["main"](), 5)
    assert json.loads(result) == case["result"]
    assert calls == [label for step in case["steps"] for label in step[0]], (
        name,
        calls,
    )
    assert position == len(case["batches"]) and not pending


def main():
    cases = json.loads((ROOT / "contract.json").read_text())["cases"]
    for name in (
        "inspection-lazy",
        "unbounded",
        "bounded-fifo",
        "getters-fifo",
        "protected-cleanup",
        "reservations",
    ):
        asyncio.run(execute(name, cases[name]))
        print(name, "CPython PASS")
    print(
        "Cancellation, failure barriers and deliberate validation limits use explicit platform traces."
    )


if __name__ == "__main__":
    main()
