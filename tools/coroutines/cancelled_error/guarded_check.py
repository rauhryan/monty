"""Narrow checks using the existing shared lock, environment and disk budgets."""

import argparse
import fcntl
import importlib.util
import json
import os
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "guard", ROOT.parent / "coordinator1/guarded_checks.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[0] == "--":
        command = command[1:]
    env = os.environ | {
        "PATH": str(Path.home() / ".cargo/bin") + ":" + os.environ["PATH"],
        "CARGO_TARGET_DIR": str(guard.build.TARGET),
        "CARGO_INCREMENTAL": "0",
        "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_PROFILE_TEST_DEBUG": "0",
        "CARGO_BUILD_JOBS": "2",
    }
    with (guard.build.ROOT / "build.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.report.exists() or args.report.with_suffix(".log").exists():
            raise RuntimeError("refusing to overwrite evidence")
        if guard.build.active_processes():
            raise RuntimeError("competing build")
        before = guard.usage()
        guard.build.require_budget(before)
        evidence = {"command": command, "before": before}
        with args.report.with_suffix(".log").open("x") as log:
            process = subprocess.Popen(
                command,
                cwd=guard.build.REPO,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                start = time.monotonic()
                while process.poll() is None:
                    guard.build.require_budget(guard.usage())
                    if time.monotonic() - start > 600:
                        raise TimeoutError("bounded check timeout")
                    time.sleep(1)
                evidence["exit"] = process.returncode
            except BaseException as error:
                evidence["stopped"] = str(error)
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                evidence["exit"] = 1
        evidence["after"] = guard.usage()
        try:
            guard.build.require_budget(evidence["after"])
        except RuntimeError as error:
            evidence["stopped"] = str(error)
            evidence["exit"] = 1
        args.report.write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence["exit"]


if __name__ == "__main__":
    raise SystemExit(main())
