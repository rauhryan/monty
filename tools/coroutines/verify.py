"""Run fixed CPython traces and actual native/Wasm processes; any required failure exits 1."""

import argparse
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path
from trace import digest, execute, finite_dumps, matches, run_case, strict_json

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
BASELINE = REPO / 'playground/coroutine-baseline'


def source_identity():
    """Include uncommitted interpreter source, including new untracked Rust files."""
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO).strip()
    diff = subprocess.check_output(
        ['git', 'diff', '--binary', 'HEAD', '--', 'crates', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'],
        cwd=REPO,
    )
    untracked = subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z', 'crates'], cwd=REPO)
    additions = {name: digest(REPO / name) for name in untracked.decode().split('\0') if name}
    return hashlib.sha256(head + b'\n' + diff + b'\n' + finite_dumps(additions, sort_keys=True).encode()).hexdigest()


def artifacts(backend):
    if backend == 'native':
        return [BASELINE / 'target/debug/coroutine-baseline']
    component = REPO / 'crates/monty-js/dist/worker/component'
    cores = sorted(component.glob('*.wasm'))
    if not cores:
        raise ValueError('missing actual Wasm cores')
    return [BASELINE / 'wasm.mjs', component / 'monty.component.js', *cores]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, default=ROOT / 'artifacts.json')
    parser.add_argument('--timeout', type=float, default=20)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('timeout must be positive')
    # Claim before running any cases; never overwrite evidence, including failed runs.
    with args.report.open('x') as output:
        report = {'status': 'FAIL', 'source_sha256': source_identity(), 'cases': {}}
        try:
            cases = strict_json((ROOT / 'cases.json').read_text())
            expected = strict_json((ROOT / 'expected.json').read_text())
            if not cases or set(cases) != set(expected):
                raise ValueError('missing/extra expectations')
            report['inputs'] = {str(p.relative_to(ROOT)): digest(p) for p in sorted(ROOT.glob('*.py'))}
            report['inputs'].update(
                {str(p.relative_to(ROOT)): digest(p) for p in sorted((ROOT / 'cases').glob('*.py'))}
            )
            report['inputs']['expected.json'] = digest(ROOT / 'expected.json')
            report['inputs']['cases.json'] = digest(ROOT / 'cases.json')
            oracle = execute([sys.executable, str(ROOT / 'oracle.py')], args.timeout)
            report['oracle'] = oracle
            if oracle.get('exit') != 0 or oracle.get('execution') != 'ran' or oracle.get('stderr'):
                raise ValueError('CPython oracle failed')
            if finite_dumps(strict_json(oracle['stdout']), sort_keys=True) != finite_dumps(expected, sort_keys=True):
                raise ValueError('CPython differs from fixed expectations')
            receipts = strict_json(args.artifacts.read_text())
            print(f'{"case":25} CPython native Wasm')
            for backend in ('native', 'wasm'):
                command = [str(BASELINE / 'target/debug/coroutine-baseline')]
                if backend == 'wasm':
                    command = [shutil.which('node') or 'node', str(BASELINE / 'wasm.mjs')]
                association_error = None
                try:
                    hashes = {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)}
                    report[backend + '_artifacts'] = hashes
                    receipt = receipts[backend]
                    if hashes != receipt['artifacts'] or report['source_sha256'] != receipt['source_sha256']:
                        raise ValueError('stale or unassociated artifact: does not implement current source')
                except (OSError, ValueError, KeyError) as error:
                    association_error = str(error)
                for name, case in cases.items():
                    directory = args.report.parent / (args.report.stem + '-cuts') / backend / name
                    directory.mkdir(parents=True, exist_ok=False)
                    trace, evidence, error = run_case(command, case, directory, args.timeout)
                    passed = matches(trace, expected[name], error) and association_error is None
                    report['cases'].setdefault(name, {})[backend] = {
                        'status': 'PASS' if passed else 'FAIL',
                        'trace': trace,
                        'execution_error': error,
                        'artifact_error': association_error,
                        'evidence': evidence,
                    }
            for name, result in report['cases'].items():
                print(f'{name:25} PASS    {result["native"]["status"]:6} {result["wasm"]["status"]}')
            if all(r[b]['status'] == 'PASS' for r in report['cases'].values() for b in ('native', 'wasm')):
                report['status'] = 'PASS'
        except (OSError, ValueError, KeyError) as error:
            report['error'] = str(error)
            print(f'FAIL: {error}')
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
