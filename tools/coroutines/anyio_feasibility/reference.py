"""Pinned independent upstream reference; compare literal traces, never write targets."""

import ast
import asyncio
import hashlib
import importlib.metadata
import json
import platform
from pathlib import Path

import anyio

ROOT = Path(__file__).resolve().parent


def frozen():
    for name, expected in json.loads(
        (ROOT / "contract.sha256.json").read_text()
    ).items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected, name
    return json.loads((ROOT / "contract.json").read_text())


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
            # Require the full externally observable boundary before settlement.
            expected = case["steps"][position]
            assert sorted(pending) == expected[1], (name, position, sorted(pending))
            position += 1
            for label, value in batch.items():
                pending.pop(label).set_result(value)
            loop.call_soon(deliver)

    async def exchange(label):
        # The effect is a cancellable checkpoint before host admission, just as a
        # native level-cancellation seam must be. No real I/O or clock is involved.
        await anyio.lowlevel.checkpoint()
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
    assert json.loads(result) == case["result"], (name, result)
    assert calls == [label for step in case["steps"] for label in step[0]], (
        name,
        calls,
    )
    assert position == len(case["batches"]) and not pending
    return {"status": "PASS", "calls": calls, "result": json.loads(result)}


def main():
    contract = frozen()
    assert platform.python_version() == contract["reference"]["python"]
    assert importlib.metadata.version("anyio") == contract["reference"]["anyio"]
    results = {}
    for name, case in contract["supported"].items():
        if "cancelled" in case or case.get("platform"):
            results[name] = {
                "status": "PLATFORM",
                "reason": "native cleanup/terminal cancellation policy; upstream requires explicit shielding",
            }
        else:
            results[name] = asyncio.run(execute(name, case))
    for name, case in (contract["scope_targets"] | contract["native_seams"]).items():
        results[name] = asyncio.run(execute(name, case))
    print(json.dumps({"pins": contract["reference"], "results": results}, indent=2))


if __name__ == "__main__":
    main()
