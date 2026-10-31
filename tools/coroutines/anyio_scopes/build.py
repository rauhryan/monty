"""Guarded unique build association for the milestone3 scope slice."""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    files = [p for p in ROOT.rglob('*') if p.is_file() and '__pycache__' not in str(p)]
    files += [
        REPO / 'crates/monty/tests/dump_compat.rs',
        *sorted((REPO / 'crates/monty/tests/fixtures').glob('*v35.bin')),
    ]
    pins = {str(p.relative_to(REPO)): digest(p) for p in files}
    subprocess.run(
        [sys.executable, str(ROOT.parent / 'anyio_feasibility/build.py'), '--output', str(a.output)], check=True
    )
    assert all(digest(REPO / p) == h for p, h in pins.items())
    association = json.loads((a.output / 'anyio-association.json').read_text())
    association['files'].update(pins)
    (a.output / 'anyio-scopes-association.json').write_text(json.dumps(association, indent=2) + '\n')


if __name__ == '__main__':
    main()
