"""Replay every literal FunctionCall and ResolveFutures cut with unchanged transports."""

import json
import subprocess


def run_case(gate, source_root, name, case, backend, directory):
    directory.mkdir()
    evidence = []
    snapshots = []
    source = source_root / "cases" / (name + ".py")
    command = (
        [str(gate.REPO / "playground/coroutine-baseline/target/debug/native_async")]
        if backend == "native"
        else ["node", str(source_root / "wasm.mjs")]
    )
    try:
        snapshot = directory / "cut-0.bin"
        raw = gate.invoke(command, ["start-step", source, snapshot], evidence)
        observed = [raw]
        batches = []
        identities = {}

        def advance(batch):
            nonlocal snapshot, raw
            gate.check(
                snapshot.is_file() and snapshot.stat().st_size > 0, "missing snapshot"
            )
            original = gate.digest(snapshot)
            snapshots.append({"path": str(snapshot), "sha256": original})
            gate.check(
                gate.invoke(command, ["dispose", snapshot], evidence)
                == {"disposed": True},
                "disposal failed",
            )
            target = directory / f"cut-{len(snapshots)}.bin"
            raw = gate.invoke(
                command, ["resume-step", snapshot, json.dumps(batch), target], evidence
            )
            branch = directory / f"branch-{len(snapshots)}.bin"
            second = gate.invoke(
                command, ["resume-step", snapshot, json.dumps(batch), branch], evidence
            )
            gate.check(
                gate.equal(raw, second) and gate.digest(snapshot) == original,
                "restore branching mismatch",
            )
            if branch.exists():
                gate.check(
                    gate.invoke(command, ["dispose", branch], evidence)
                    == {"disposed": True},
                    "branch disposal failed",
                )
            batches.append(batch)
            observed.append(raw)
            snapshot = target

        for step, scripted in zip(case["steps"], case["batches"], strict=True):
            for label in step[0]:
                gate.check(
                    set(raw) == {"call"} and set(raw["call"]) == {"id", "label"},
                    "expected literal host call",
                )
                call = raw["call"]
                gate.check(
                    type(call["id"]) is int and call["label"] == label,
                    "host request order changed",
                )
                gate.check(call["id"] not in identities, "reused call id")
                identities[call["id"]] = label
                advance({"ack": True})
            gate.check(
                gate.equal(gate.view(raw, identities), [[], step[1]]),
                "pending calls changed",
            )
            by_label = {label: ident for ident, label in identities.items()}
            batch = {
                "results": [
                    {"id": by_label[label], "value": value}
                    for label, value in scripted.items()
                    if label != "$cancel"
                ]
            }
            if "$cancel" in scripted:
                batch["cancel"] = scripted["$cancel"]
            advance(batch)
        gate.terminal(raw, case)
        batch_file = directory / "batches.json"
        batch_file.write_text(json.dumps(batches) + "\n")
        continuous = gate.invoke(command, ["run-step", source, batch_file], evidence)
        gate.check(
            gate.equal(continuous, observed), "uninterrupted versus restored mismatch"
        )
        return {"status": "PASS", "cuts": len(snapshots)}
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        return {"status": "FAIL", "reason": str(error)}
    finally:
        (directory / "evidence.json").write_text(
            json.dumps({"commands": evidence, "snapshots": snapshots}, indent=2) + "\n"
        )
