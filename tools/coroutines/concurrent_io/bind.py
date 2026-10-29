"""Reuse the existing guarded native/Wasm build and extend its source receipt."""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def digest(path):
    """Hash the exact file consumed by execution."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    """Build before associating artifacts; refuse source drift or reused evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    sources = {str(p.relative_to(REPO)): digest(p) for p in ROOT.iterdir() if p.is_file()}
    for name in (
        'crates/monty/tests/generator_runtime.rs',
        'crates/monty/tests/unit/generator_ownership.rs',
        'crates/monty/tests/fixtures/dump_v24.bin',
        'crates/monty/tests/fixtures/dump_v25.bin',
        'crates/monty/tests/fixtures/dump_v26.bin',
    ):
        test = REPO / name
        sources[name] = digest(test)
    subprocess.run(
        [
            sys.executable,
            str(ROOT.parent / 'native1/bind.py'),
            '--output',
            str(args.output),
        ],
        check=True,
    )
    assert sources == {name: digest(REPO / name) for name in sources}
    association = json.loads((args.output / 'build-association.json').read_text())
    association['files'].update(sources)
    binary = REPO / 'playground/coroutine-baseline/target/debug/concurrent_io'
    association['artifacts'][str(binary.relative_to(REPO))] = digest(binary)
    (args.output / 'concurrent-association.json').write_text(json.dumps(association, indent=2) + '\n')


if __name__ == '__main__':
    main()
