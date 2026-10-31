"""Additive protected-gather regressions using the unchanged native/Wasm transports."""

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
        "native_async_gate", ROOT.parent / "native_async/verify.py"
    )
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    gate.association(args.association)
    gate.frozen()
    before = gate.budget()
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "cases").mkdir()
    original_root = gate.ROOT
    (args.output / "wasm.mjs").symlink_to(original_root / "wasm.mjs")
    gate.ROOT = args.output.resolve()
    source = (ROOT / "cases/cleanup-gather-error.py").read_text()
    # The first program is byte-identical to the independent review repro.
    programs = {
        "cleanup-gather-error": source,
        "reversed-cleanup-gather-error": source,
        "nested-cleanup-gather-error": source.replace(
            "async def main():",
            "async def inner():\n    await asyncio.gather(left(), right())\n\nasync def main():",
        ).replace(
            "            await asyncio.gather(left(), right())",
            "            await asyncio.gather(inner())",
        ),
        "two-cleanup-errors": source.replace(
            "        await exchange('child-release')",
            "        await exchange('child-release')\n        raise RuntimeError('right-error')",
        ),
        "direct-external-sibling": source.replace(
            "asyncio.gather(left(), right())", "asyncio.gather(left(), exchange('B'))"
        ),
        "commit-error-after-child-admission": source.replace(
            "async def main():",
            "async def done():\n    return None\n\nasync def main():",
        ).replace(
            "            await asyncio.gather(left(), right())",
            "            used = done()\n            await used\n            await asyncio.gather(right(), used)",
        ),
        "normal-cleanup-gather-error": source.replace(
            "async def main():", "async def work():"
        ).replace(
            "asyncio.run(main())",
            "async def main():\n    try:\n        await work()\n    except ValueError as e:\n        return json.dumps(str(e))\n\nasyncio.run(main())",
        ),
    }
    report = {
        "results": {},
        "association_sha256": gate.digest(args.association),
        "disk_before": before,
    }
    for name, program in programs.items():
        (args.output / "cases" / (name + ".py")).write_text(program)
        direct = name == "direct-external-sibling"
        commit = name == "commit-error-after-child-admission"
        steps = [
            [["body"], ["body"]],
            [["A", "B"], ["A", "B"]],
            [[], ["B"]],
        ]
        batches = [
            {"$cancel": "FIRST"},
            {"A": "a", "$cancel": "REPEAT"},
            {"B": "b", "$cancel": "AGAIN"},
        ]
        if not direct:
            steps.append([["child-release"], ["child-release"]])
            batches.append({"child-release": "c", "$cancel": "AGAIN"})
        steps.append([["parent-release"], ["parent-release"]])
        batches.append({"parent-release": "p", "$cancel": "LAST"})
        case = {"steps": steps, "batches": batches, "cancelled": "FIRST"}
        if name == "cleanup-gather-error":
            for batch in case["batches"][1:]:
                batch.pop("$cancel", None)
        if direct:
            case["steps"][1] = [["B", "A"], ["A", "B"]]
        if name == "reversed-cleanup-gather-error":
            case["steps"][2] = [["child-release"], ["A", "child-release"]]
            case["steps"][3] = [[], ["child-release"]]
            case["batches"][1] = {"B": "b", "$cancel": "REPEAT"}
            case["batches"][2] = {"A": "a", "$cancel": "AGAIN"}
        if commit:
            case["steps"][1] = [["B"], ["B"]]
            del case["steps"][2]
            del case["batches"][1]
        if name == "normal-cleanup-gather-error":
            del case["cancelled"]
            case["result"] = "left-error"
            for batch in case["batches"]:
                batch.pop("$cancel", None)
            case["batches"][0] = {"body": "body"}
        for backend in ("native", "wasm"):
            folder = args.output / (backend + "-" + name)
            result = gate.run_case(name, case, backend, folder)
            if result["status"] == "PASS":
                evidence = json.loads((folder / "evidence.json").read_text())
                terminal = json.loads(evidence["commands"][-1]["stdout"])[-1][
                    "complete"
                ]
                try:
                    if "cancelled" in case:
                        expected = (
                            "RuntimeError: cannot reuse already awaited coroutine"
                            if commit
                            else "ValueError: left-error"
                        )
                        gate.check(
                            terminal["error"].endswith(expected),
                            "lost first cleanup error",
                        )
                        gate.check(
                            terminal["value_export_error"] is None,
                            "failed terminal export",
                        )
                except (ValueError, AttributeError, KeyError) as error:
                    result = {"status": "FAIL", "reason": str(error)}
            report["results"][backend + "/" + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = original_root
    gate.association(args.association)
    gate.frozen()
    report["disk_after"] = gate.budget()
    report["status"] = (
        "PASS"
        if all(x["status"] == "PASS" for x in report["results"].values())
        else "FAIL"
    )
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return int(report["status"] != "PASS")


if __name__ == "__main__":
    raise SystemExit(main())
