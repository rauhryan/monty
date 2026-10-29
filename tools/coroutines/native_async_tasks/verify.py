"""Additive public Task traces using unchanged native/Wasm transports and checker."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--association", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location(
        "gate", ROOT.parent / "native_async/verify.py"
    )
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    gate.association(args.association)
    gate.frozen()
    before = gate.budget()
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "cases").mkdir()
    (args.output / "wasm.mjs").symlink_to(gate.ROOT / "wasm.mjs")
    programs = {
        "shared-protected": (
            [
                [["S", "A", "B"], ["A", "B", "S"]],
                [[], ["B", "S"]],
                [[], ["S"]],
                [["A-release", "B-release"], ["A-release", "B-release"]],
                [[], ["B-release"]],
            ],
            [
                {"A": "a"},
                {"B": "b"},
                {"S": "s"},
                {"A-release": "a"},
                {"B-release": "b"},
            ],
        ),
        "prestart": ([[["inspect"], ["inspect"]]], [{"inspect": "i"}]),
        "joins": (
            [[["A", "B"], ["A", "B"]], [[], ["A"]]],
            [{"B": "b"}, {"A": "a"}],
        ),
        "lifetime": (
            [[["A", "B"], ["A", "B"]], [["finish"], ["B", "finish"]], [[], ["finish"]]],
            [{"A": "a"}, {"B": "b"}, {"finish": "f"}],
        ),
        "multi-errors": (
            [[["A"], ["A"]], [["inspect"], ["inspect"]]],
            [{"A": "a"}, {"inspect": "i"}],
        ),
        "errors": (
            [[["A"], ["A"]], [["inspect"], ["inspect"]]],
            [{"A": "a"}, {"inspect": "i"}],
        ),
        "cancel": (
            [
                [["control", "A"], ["A", "control"]],
                [["again", "release"], ["again", "release"]],
                [[], ["release"]],
                [["waiter-release"], ["waiter-release"]],
            ],
            [
                {"control": "c"},
                {"again": "g", "A": "late"},
                {"release": "r"},
                {"waiter-release": "w"},
            ],
        ),
        "caught": (
            [[["control", "A"], ["A", "control"]], [["recover"], ["recover"]]],
            [{"control": "c"}, {"recover": "r"}],
        ),
        "cleanup": (
            [
                [["body"], ["body"]],
                [["A", "B"], ["A", "B"]],
                [[], ["B"]],
                [["child-release"], ["child-release"]],
                [["parent-release"], ["parent-release"]],
            ],
            [
                {"body": "ok"},
                {"A": "a"},
                {"B": "b"},
                {"child-release": "r"},
                {"parent-release": "p"},
            ],
        ),
        "cancel-gather": (
            [
                [["control", "A", "B"], ["A", "B", "control"]],
                [["A-release", "B-release"], ["A-release", "B-release"]],
                [[], ["B-release"]],
                [["owner-release"], ["owner-release"]],
            ],
            [
                {"control": "c"},
                {"A-release": "a"},
                {"B-release": "b"},
                {"owner-release": "p"},
            ],
        ),
    }
    programs["cancel-error"] = programs["cancel"]
    programs["cancel-gather-wrapped"] = (
        [
            [["control", "A", "B"], ["A", "B", "control"]],
            [["B-release", "A-release"], ["A-release", "B-release"]],
            [[], ["B-release"]],
            [["owner-release"], ["owner-release"]],
        ],
        [
            {"control": "c"},
            {"A-release": "a"},
            {"B-release": "b"},
            {"owner-release": "p"},
        ],
    )
    for name in programs:
        (args.output / "cases" / (name + ".py")).write_bytes(
            (ROOT / "cases" / (name + ".py")).read_bytes()
        )
    gate.ROOT = args.output.resolve()
    report = {
        "results": {},
        "inputs": {
            str(path.relative_to(REPO)): gate.digest(path)
            for path in ROOT.rglob("*")
            if path.is_file() and "__pycache__" not in str(path)
        },
        "association_sha256": gate.digest(args.association),
        "disk_before": before,
    }
    for backend in ("native", "wasm"):
        for name, (steps, batches) in programs.items():
            result = gate.run_case(
                name,
                {"steps": steps, "batches": batches, "result": "ok"},
                backend,
                args.output / (backend + "-" + name),
            )
            report["results"][backend + "/" + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = ROOT.parent / "native_async"
    gate.association(args.association)
    gate.frozen()
    report["disk_after"] = gate.budget()
    report["status"] = (
        "PASS"
        if all(r["status"] == "PASS" for r in report["results"].values())
        else "FAIL"
    )
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return int(report["status"] != "PASS")


if __name__ == "__main__":
    raise SystemExit(main())
