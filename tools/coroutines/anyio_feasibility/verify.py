"""Verify the supported slice and report unresolved scope targets separately."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def frozen(gate):
    for name, expected in gate.strict_json(
        (ROOT / "contract.sha256.json").read_text()
    ).items():
        gate.check(
            gate.digest(ROOT / name) == expected, "frozen AnyIO input changed: " + name
        )
    contract = gate.strict_json((ROOT / "contract.json").read_text())
    for path in (ROOT / "cases").glob("*.py"):
        text = path.read_text()
        gate.check(
            "__monty_host__" not in text and "._native" not in text,
            "private guest entry",
        )
    return contract


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--association", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    gate = module("gate", ROOT.parent / "native_async/verify.py")
    groups = module("groups", ROOT.parent / "native_async_groups/verify.py")
    steps = module("steps", ROOT.parent / "native_async_queues/steps.py")
    gate.self_check()
    groups.self_check(gate)
    contract = frozen(gate)
    receipt_hash = gate.digest(args.association)
    receipt = gate.association(args.association)
    for path in ROOT.rglob("*"):
        if path.is_file() and "__pycache__" not in str(path):
            gate.check(
                receipt["files"].get(str(path.relative_to(gate.REPO)))
                == gate.digest(path),
                "unbound AnyIO input",
            )
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    (output / "wasm.mjs").symlink_to(gate.ROOT / "wasm.mjs")
    (output / "cases").mkdir()
    for path in (ROOT / "cases").glob("*.py"):
        (output / "cases" / path.name).write_bytes(path.read_bytes())
    report = {
        "supported": {},
        "scope_capabilities": {},
        "native_seams": {},
        "disk_before": gate.budget(),
    }
    for backend in ("native", "wasm"):
        for name, case in (contract["supported"] | contract["native_seams"]).items():
            directory = output / (backend + "-" + name)
            result = steps.run_case(gate, output, name, case, backend, directory)
            if result["status"] == "PASS" and "cancelled" in case:
                try:
                    groups.check_evidence(
                        gate.strict_json((directory / "evidence.json").read_text()),
                        "host-cancel",
                        gate,
                    )
                except (ValueError, KeyError, TypeError) as error:
                    result = {"status": "FAIL", "reason": str(error)}
            report["native_seams" if name in contract["native_seams"] else "supported"][
                backend + "/" + name
            ] = result
            print(backend, name, result, flush=True)
        command = (
            [str(gate.REPO / "playground/coroutine-baseline/target/debug/native_async")]
            if backend == "native"
            else ["node", str(output / "wasm.mjs")]
        )
        for name in contract["scope_targets"]:
            evidence = []
            try:
                snapshot = output / (backend + "-" + name + ".bin")
                raw = gate.invoke(
                    command,
                    ["start-step", output / "cases" / (name + ".py"), snapshot],
                    evidence,
                )
                gate.check(
                    gate.equal(raw, {"complete": contract["scope_capability_result"]}),
                    "unexpected scope capability",
                )
                gate.check(not snapshot.exists(), "unsupported scope must not suspend")
                result = {"status": "EXPECTED_UNSUPPORTED", "target_satisfied": False}
            except (ValueError, OSError) as error:
                result = {
                    "status": "FAIL",
                    "reason": str(error),
                    "target_satisfied": False,
                }
            report["scope_capabilities"][backend + "/" + name] = result
            (output / (backend + "-" + name + "-evidence.json")).write_text(
                json.dumps(evidence, indent=2) + "\n"
            )
            print(backend, name, result, flush=True)
    frozen(gate)
    gate.association(args.association)
    gate.check(gate.digest(args.association) == receipt_hash, "association drift")
    report["association_sha256"] = receipt_hash
    report["disk_after"] = gate.budget()
    report["status"] = (
        "PASS"
        if all(
            r["status"] == "PASS"
            for r in (report["supported"] | report["native_seams"]).values()
        )
        and all(
            r["status"] == "EXPECTED_UNSUPPORTED"
            for r in report["scope_capabilities"].values()
        )
        else "FAIL"
    )
    report["anyio_scope_acceptance"] = "UNSUPPORTED: 0/6 target coordinates"
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "supported_slice": report["status"],
                "scope_acceptance": report["anyio_scope_acceptance"],
            }
        )
    )
    return int(report["status"] != "PASS")


if __name__ == "__main__":
    raise SystemExit(main())
