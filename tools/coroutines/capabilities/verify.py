"""Verify pinned artifacts, run bounded proofs, and hash new inputs separately from old bindings."""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
sys.path.insert(0, str(ROOT.parent / 'coordinator1'))
sys.path.insert(0, str(ROOT.parent / 'native1'))
import guarded_checks  # noqa: E402
from bind import inputs  # noqa: E402

BINDING = REPO / 'playground/coroutine-implementation/protocol-repair3/build/binding.json'
PIN = '67de000ec147cf127d725f719a2f41bd620bf1c9f276d1fd651c21af52bd3097'


def digest(path):
    """Hash actual bytes, never rewrite the original artifact binding."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def authenticate(binding_path=BINDING, pin=PIN):
    """Authenticate artifacts/shims, runtime inputs and disk against existing receipts."""
    assert digest(binding_path) == pin
    binding = json.loads(binding_path.read_text())
    for name, expected in binding['files'].items():
        assert digest(REPO / name) == expected, name
    current = inputs()
    assert (
        hashlib.sha256(json.dumps(current['runtime'], sort_keys=True).encode()).hexdigest() == binding['runtime_sha256']
    )
    assert current['build_inputs'] == binding['build_inputs']
    disk = guarded_checks.usage()
    guarded_checks.build.require_budget(disk)
    return disk


def main():
    """Capture commands, outcomes, source preservation and new-input identities."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--inventory', required=True, type=Path)
    parser.add_argument('--binding', type=Path, default=BINDING)
    parser.add_argument('--binding-sha256', default=PIN)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    report = {
        'binding_sha256': args.binding_sha256,
        'before': authenticate(args.binding, args.binding_sha256),
        'commands': [],
    }
    files = list(ROOT.glob('*.py')) + list(ROOT.glob('*.md')) + list((ROOT.parent / 'stepping').glob('*.py'))
    report['inputs'] = {str(p.relative_to(REPO)): digest(p) for p in sorted(files)}
    commands = [
        [sys.executable, str(ROOT / 'test_journal.py')],
        *[
            [sys.executable, str(ROOT / 'demo.py'), '--backend', backend, '--output', str(args.output / backend)]
            for backend in ('native', 'wasm')
        ],
        [str(REPO / '.venv/bin/ruff'), 'check', '--no-force-exclude', *map(str, ROOT.glob('*.py'))],
        [str(REPO / '.venv/bin/ruff'), 'format', '--check', '--no-force-exclude', *map(str, ROOT.glob('*.py'))],
        ['git', 'diff', '--check'],
    ]
    for command in commands:
        result = subprocess.run(command, cwd=REPO, text=True, capture_output=True, timeout=120)
        report['commands'].append(
            {'argv': command, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}
        )
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if result.returncode:
            raise RuntimeError(report['commands'][-1])
    for name, expected in report['inputs'].items():
        assert digest(REPO / name) == expected, name
    inventory = json.loads(args.inventory.read_text())
    for name, expected in inventory['files'].items():
        assert digest(REPO / name) == expected, name
    report['preserved_files'] = len(inventory['files'])
    report['inventory_sha256'] = digest(args.inventory)
    report['after'] = authenticate(args.binding, args.binding_sha256)
    report['evidence'] = {
        str(p.relative_to(args.output)): digest(p)
        for p in args.output.rglob('*')
        if p.is_file() and p != args.output / 'report.json'
    }
    report['status'] = 'PASS'
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': 'PASS', 'report': str(args.output / 'report.json')}))


if __name__ == '__main__':
    main()
