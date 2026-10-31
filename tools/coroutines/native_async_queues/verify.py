"""Public Queue traces using unchanged transports and strict terminal checking."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--association", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native-preflight", action="store_true")
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location(
        "gate", ROOT.parent / "native_async/verify.py"
    )
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    if not args.native_preflight:
        if args.association is None:
            parser.error("--association required")
        gate.association(args.association)
    gate.frozen()
    pins = {
        str(p.relative_to(gate.REPO)): gate.digest(p)
        for p in ROOT.rglob("*")
        if p.is_file() and "__pycache__" not in str(p)
    }
    before = gate.budget()
    args.output.mkdir(parents=True)
    (args.output / "wasm.mjs").symlink_to(gate.ROOT / "wasm.mjs")
    gate.ROOT = ROOT
    # Keep the frozen adapter path while cases remain owning-suite inputs.
    (args.output / "cases").mkdir()
    for path in (ROOT / "cases").glob("*.py"):
        (args.output / "cases" / path.name).write_bytes(path.read_bytes())
    gate.ROOT = args.output.resolve()
    step_spec = importlib.util.spec_from_file_location("queue_steps", ROOT / "steps.py")
    steps = importlib.util.module_from_spec(step_spec)
    step_spec.loader.exec_module(steps)
    cases = gate.strict_json((ROOT / "contract.json").read_text())["cases"]
    report = {
        "results": {},
        "disk_before": before,
        "associated": not args.native_preflight,
    }
    for backend in ("native",) if args.native_preflight else ("native", "wasm"):
        for name, case in cases.items():
            directory = args.output / (backend + "-" + name)
            result = steps.run_case(
                gate, args.output.resolve(), name, case, backend, directory
            )
            if result["status"] == "PASS" and "cancelled" in case:
                group_spec = importlib.util.spec_from_file_location(
                    "groups", ROOT.parent / "native_async_groups/verify.py"
                )
                groups = importlib.util.module_from_spec(group_spec)
                group_spec.loader.exec_module(groups)
                try:
                    evidence = gate.strict_json(
                        (directory / "evidence.json").read_text()
                    )
                    for command in evidence["commands"]:
                        if command["exit"] == 0:
                            for terminal in groups.terminals(
                                gate.strict_json(command["stdout"])
                            ):
                                gate.check(
                                    gate.equal(
                                        terminal["interrupted_error"],
                                        case.get("interrupted"),
                                    ),
                                    "unexpected interrupted error",
                                )
                                normalized = dict(terminal, interrupted_error=None)
                                groups.check_terminal(normalized, "host-cancel", gate)
                except (ValueError, KeyError, TypeError) as error:
                    result = {"status": "FAIL", "reason": str(error)}
            report["results"][backend + "/" + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = ROOT.parent / "native_async"
    if not args.native_preflight:
        gate.association(args.association)
        report["association_sha256"] = gate.digest(args.association)
    gate.frozen()
    report["disk_after"] = gate.budget()
    report["status"] = (
        "PASS"
        if all(r["status"] == "PASS" for r in report["results"].values())
        else "FAIL"
    )
    report["source_pins"] = {
        str(p.relative_to(gate.REPO)): gate.digest(p)
        for p in ROOT.rglob("*")
        if p.is_file() and "__pycache__" not in str(p)
    }
    gate.check(gate.equal(pins, report["source_pins"]), "suite source drift")
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return int(report["status"] != "PASS")


if __name__ == "__main__":
    raise SystemExit(main())
