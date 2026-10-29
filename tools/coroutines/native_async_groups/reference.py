"""CPython reference for supported standard group semantics, with literal scripts."""

import ast
import asyncio
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


async def execute(name, batches, expected_calls):
    pending = {}
    calls = []
    position = 0
    loop = asyncio.get_running_loop()

    def deliver():
        nonlocal position
        if position == len(batches):
            return
        batch = batches[position]
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
    assert json.loads(result) == "ok"
    assert calls == expected_calls, (name, calls)
    assert position == len(batches) and not pending


if __name__ == "__main__":
    for name, batches, calls in [
        (
            "nested-caught",
            [
                {"A": "a"},
                {"child-release": "r"},
                {"after-inner": "i"},
                {"O": "o"},
                {"parent-release": "p"},
            ],
            ["body", "O", "A", "B", "child-release", "after-inner", "parent-release"],
        ),
        (
            "normal",
            [{"B": "b"}, {"A": "a"}, {"discarded": "d"}, {"parent-release": "p"}],
            ["A", "discarded", "B", "parent-release"],
        ),
        (
            "same-owner",
            [{"B": "b"}, {"after-inner": "i"}, {"A": "a"}, {"parent-release": "p"}],
            ["A", "B", "after-inner", "parent-release"],
        ),
        (
            "failure",
            [{"A": "a"}, {"child-release": "r"}, {"parent-release": "p"}],
            ["body", "A", "B", "child-release", "parent-release"],
        ),
    ]:
        asyncio.run(execute(name, batches, calls))
        print(name, "CPython PASS")
    print(
        "Protected cleanup, sticky host cancellation and durable ordinary-gather lifetimes are platform contracts."
    )
    print(
        "ExceptionGroup typing is explicitly unsupported; portable failure.py inspects the documented exception tuple fallback."
    )
    for name in ("root-shutdown", "root-error"):
        calls = []

        async def root_exchange(label):
            calls.append(label)
            future = asyncio.get_running_loop().create_future()
            if label != "body":
                asyncio.get_running_loop().call_soon(future.set_result, "reply")
            return await future

        path = ROOT / "cases" / (name + ".py")
        module = ast.parse(path.read_text(), filename=str(path))
        last = module.body.pop()
        namespace = {"exchange": root_exchange}
        exec(compile(module, str(path), "exec"), namespace)
        result = eval(compile(ast.Expression(last.value), str(path), "eval"), namespace)
        assert json.loads(result) == "ok"
        assert calls == ["control", "body", "child-release"], calls
        print(name, "CPython PASS")
