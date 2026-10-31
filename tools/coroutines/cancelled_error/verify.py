"""Literal cancellation-type checks on native/Wasm and every genuine restore cut."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "gate", ROOT.parent / "native_async/verify.py"
)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
original_root = gate.ROOT
gate.ROOT = ROOT
base_terminal = gate.terminal
base_invoke = gate.invoke


def invoke(command, arguments, evidence):
    command = [
        str(ROOT / "wasm.mjs") if str(item) == str(original_root / "wasm.mjs") else item
        for item in command
    ]
    return base_invoke(command, arguments, evidence)


gate.invoke = invoke


def terminal(raw, case):
    base_terminal(raw, case)
    if "cancelled" in case:
        gate.check(
            raw["complete"]
            == {
                "status": "cancelled",
                "reason": "FIRST",
                "value": '"suppressed"',
                "error": None,
                "interrupted_error": None,
                "value_export_error": None,
            },
            "cancellation terminal changed",
        )


gate.terminal = terminal
CASES = {
    "host": {
        "steps": [[["body"], ["body"]], [["cleanup"], ["cleanup"]], [[], ["cleanup"]]],
        "batches": [{"$cancel": "FIRST"}, {"$cancel": "SECOND"}, {"cleanup": "ok"}],
        "cancelled": "FIRST",
    },
    "scope": {
        "steps": [[["cleanup"], ["cleanup"]], [["after"], ["after"]]],
        "batches": [{"cleanup": "ok"}, {"after": "ok"}],
        "result": "ok",
    },
    "task": {
        "steps": [[["control"], ["control"]], [["cleanup"], ["cleanup"]]],
        "batches": [{"control": "ok"}, {"cleanup": "ok"}],
        "result": "ok",
    },
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--association", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    gate.ROOT = original_root
    gate.association(args.association)
    gate.ROOT = ROOT
    pins = {
        str(p): gate.digest(p)
        for p in ROOT.rglob("*")
        if p.is_file() and "__pycache__" not in str(p)
    }
    results = {}
    for backend in ("native", "wasm"):
        for name, case in CASES.items():
            result = gate.run_case(
                name, case, backend, args.output / (backend + "-" + name)
            )
            results[backend + "/" + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = original_root
    gate.terminal = base_terminal
    for backend in ("native", "wasm"):
        for name, case in gate.frozen().items():
            result = gate.run_case(
                name, case, backend, args.output / ("baseline-" + backend + "-" + name)
            )
            results["baseline/" + backend + "/" + name] = result
            print("baseline", backend, name, result, flush=True)
    gate.association(args.association)
    gate.ROOT = ROOT
    assert all(gate.digest(Path(p)) == h for p, h in pins.items())
    (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
    return int(any(r["status"] != "PASS" for r in results.values()))


if __name__ == "__main__":
    raise SystemExit(main())
