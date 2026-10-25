"""Run focused source checks through the serialized build/budget guard."""

import argparse
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('backend', choices=['native', 'wasm', 'generator-compile-tests', 'generator-metadata-tests'])
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    return subprocess.call(
        [
            'python3',
            str(REPO / 'playground/coroutine-baseline/build.py'),
            (args.backend if args.backend.endswith('-tests') else args.backend + '-check'),
            '--report',
            str(args.report),
        ],
        cwd=REPO,
    )


if __name__ == '__main__':
    raise SystemExit(main())
