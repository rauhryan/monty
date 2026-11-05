"""Additive authored-cancellation checks; reuse the unchanged restore comparator."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "cancellation", ROOT.parent / "cancelled_error/verify.py"
)
cancellation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cancellation)
gate = cancellation.gate
CASES = {
    name: {
        "steps": [[[label], [label]] for label in labels],
        "batches": [{label: "ok"} for label in labels],
        "result": "ok",
    }
    for name, labels in {
        "tasks": ["before", "completed"],
        "groups": ["asyncio", "anyio", "drained"],
        "scopes": ["before", "completed"],
    }.items()
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--association", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    gate.ROOT = cancellation.original_root
    gate.association(args.association)
    pins = {
        str(p): gate.digest(p)
        for p in ROOT.rglob("*")
        if p.is_file() and "__pycache__" not in str(p)
    }
    for name in ("verify.py", "wasm.mjs"):
        path = cancellation.ROOT / name
        pins[str(path)] = gate.digest(path)
    (args.output / "inputs.json").write_text(json.dumps(pins, indent=2) + "\n")
    gate.ROOT = ROOT
    # The adapter stays the authenticated cancellation-contract-8 adapter.
    original_invoke = gate.invoke

    def invoke(command, arguments, evidence):
        command = [
            str(cancellation.ROOT / "wasm.mjs")
            if item == str(ROOT / "wasm.mjs")
            else item
            for item in command
        ]
        return original_invoke(command, arguments, evidence)

    gate.invoke = invoke
    results = {}
    for backend in ("native", "wasm"):
        for name, case in CASES.items():
            result = gate.run_case(
                name, case, backend, args.output / (backend + "-" + name)
            )
            results[backend + "/" + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = cancellation.original_root
    gate.association(args.association)
    assert all(gate.digest(Path(p)) == h for p, h in pins.items())
    (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
    return int(any(r["status"] != "PASS" for r in results.values()))


if __name__ == "__main__":
    raise SystemExit(main())
