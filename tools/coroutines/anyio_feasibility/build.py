"""Associate this slice with a fresh guarded native and actual-Wasm build."""

import argparse
import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--reuse-association",
        type=Path,
        help="Authenticate an unchanged runtime build when only verifier inputs changed",
    )
    args = parser.parse_args()
    paths = [p for p in ROOT.rglob("*") if p.is_file() and "__pycache__" not in str(p)]
    paths += [
        REPO / "crates/monty/tests/generator_runtime.rs",
        ROOT.parent / "native_async_queues/steps.py",
        ROOT.parent / "native_async_groups/verify.py",
    ]
    paths += [
        REPO / name
        for name in (
            "docs/limitations/anyio.md",
            "docs/limitations/index.md",
            "docs/limitations/modules.md",
            "mkdocs.yml",
        )
    ]
    pins = {str(p.relative_to(REPO)): digest(p) for p in paths}
    if args.reuse_association:
        spec = importlib.util.spec_from_file_location(
            "native_gate", ROOT.parent / "native_async/verify.py"
        )
        gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gate)
        association = gate.association(args.reuse_association)
        args.output.mkdir(parents=True, exist_ok=False)
        key = str(args.reuse_association.resolve().relative_to(REPO))
        association["files"][key] = digest(args.reuse_association)
        association["reused_build_association"] = {
            "path": key,
            "sha256": digest(args.reuse_association),
        }
    else:
        subprocess.run(
            [
                sys.executable,
                str(ROOT.parent / "native_async/build.py"),
                "--output",
                str(args.output),
            ],
            check=True,
        )
        association = json.loads(
            (args.output / "native-async-association.json").read_text()
        )
    assert all(digest(REPO / p) == h for p, h in pins.items()), (
        "slice changed during build"
    )
    association["files"].update(pins)
    association["source_head"] = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True
    ).strip()
    association["source_status"] = subprocess.check_output(
        ["git", "status", "--short"], cwd=REPO, text=True
    )
    (args.output / "anyio-association.json").write_text(
        json.dumps(association, indent=2) + "\n"
    )
    print(args.output / "anyio-association.json")


if __name__ == "__main__":
    main()
