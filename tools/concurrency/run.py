"""Build and replay the maintained concurrency checks using tracked inputs only."""

import argparse
import hashlib
import importlib.util
import json
import shutil
from pathlib import Path

import guard

ROOT = Path(__file__).resolve().parent
REPO = guard.REPO
LEGACY = REPO / 'tools/coroutines'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files_under(directory):
    return [
        p for p in directory.rglob('*') if p.is_file() and '__pycache__' not in p.parts
    ]


def sources():
    """Independent of Git metadata; also works in a tracked-source export."""
    paths = [REPO / 'Cargo.toml', REPO / 'Cargo.lock']
    paths.extend(files_under(REPO / '.cargo'))
    for crate in (REPO / 'crates').iterdir():
        if not crate.is_dir():
            continue
        for name in ('Cargo.toml', 'build.rs'):
            if (crate / name).is_file():
                paths.append(crate / name)
        for name in ('src', 'wit', 'proto'):
            paths.extend(files_under(crate / name))
    paths.extend(files_under(ROOT))
    for name in ('native_async', 'native1', 'cancelled_error', 'authored_cancellation'):
        paths.extend(files_under(LEGACY / name))
    paths.extend(
        [
            REPO / 'crates/monty-js/scripts/build-component.mjs',
            REPO / 'crates/monty-js/package.json',
            REPO / 'crates/monty-js/package-lock.json',
        ]
    )
    return {str(p.relative_to(REPO)): digest(p) for p in sorted(set(paths))}


def artifacts():
    paths = [
        guard.TARGET / 'debug/native_async',
        guard.TARGET / 'wasm32-wasip1/release/monty_wasm_runtime.wasm',
    ]
    component = REPO / 'crates/monty-js/dist/worker/component'
    paths.extend(component.glob('*.js'))
    paths.extend(component.glob('*.wasm'))
    paths.extend(
        (
            REPO / 'crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist'
        ).rglob('*.js')
    )
    required = [
        component / 'monty.component.js',
        component / 'monty.component.core.wasm',
    ]
    if not all(p.is_file() for p in required):
        raise RuntimeError('missing generated Wasm component; run build')
    return {str(p.relative_to(REPO)): digest(p) for p in sorted(set(paths))}


def authenticate(path):
    association = json.loads(path.read_text())
    if association['sources'] != sources() or association['artifacts'] != artifacts():
        raise RuntimeError(
            'source or artifact inventory differs from the build association'
        )
    for receipt, expected in association['receipts'].items():
        if digest(path.parent / receipt) != expected:
            raise RuntimeError('build receipt changed: ' + receipt)
    return association


def preflight():
    required = [
        ROOT / 'native/Cargo.toml',
        ROOT / 'native/Cargo.lock',
        LEGACY / 'native_async/native.rs',
        LEGACY / 'native_async/verify.py',
        LEGACY / 'native1/bind.py',
        LEGACY / 'cancelled_error/wasm.mjs',
        LEGACY / 'cancelled_error/verify.py',
        LEGACY / 'authored_cancellation/verify.py',
    ]
    for path in required:
        if not path.is_file():
            raise RuntimeError('missing tracked input: ' + str(path))
    cancellation = load('cancellation', LEGACY / 'cancelled_error/verify.py')
    gate = cancellation.gate
    gate.ROOT = cancellation.original_root
    gate.self_check()
    return cancellation


def build(output):
    before = sources()
    steps = {
        'native': [
            'cargo',
            'build',
            '--locked',
            '--manifest-path',
            str(ROOT / 'native/Cargo.toml'),
            '--bin',
            'native_async',
        ],
        'wasm': [
            'cargo',
            'build',
            '--locked',
            '-p',
            'monty-wasm-runtime',
            '--target',
            'wasm32-wasip1',
            '--release',
        ],
        'component': ['node', 'crates/monty-js/scripts/build-component.mjs'],
    }
    receipts = {}
    for name, command in steps.items():
        print('Building ' + name, flush=True)
        receipt = output / (name + '.json')
        guard.execute(command, receipt)
        receipts[receipt.name] = digest(receipt)
    if before != sources():
        raise RuntimeError('source changed during build')
    association = output / 'association.json'
    association.write_text(
        json.dumps(
            {
                'sources': before,
                'artifacts': artifacts(),
                'receipts': receipts,
                'tools': {
                    name: shutil.which(name, path=guard.environment()['PATH'])
                    for name in ('python3', 'node', 'cargo')
                },
            },
            indent=2,
        )
        + '\n'
    )
    return association


def verify(association, output, cancellation):
    """Reuse fixed comparisons unchanged; redirect only executable selection."""
    authenticate(association)
    gate = cancellation.gate
    gate.ROOT = cancellation.original_root
    frozen = gate.frozen()
    authored = load('authored', LEGACY / 'authored_cancellation/verify.py')
    suites = [
        ('native_async', frozen),
        ('cancelled_error', cancellation.CASES),
        ('authored_cancellation', authored.CASES),
    ]
    invoke = cancellation.base_invoke

    def invoke_adapter(command, arguments, evidence):
        if command[0] == 'node':
            command = ['node', str(LEGACY / 'cancelled_error/wasm.mjs')]
        else:
            command = [str(guard.TARGET / 'debug/native_async')]
        return invoke(command, arguments, evidence)

    gate.invoke = invoke_adapter
    results = {}
    for suite, cases in suites:
        gate.ROOT = LEGACY / suite
        gate.terminal = (
            cancellation.terminal
            if suite == 'cancelled_error'
            else cancellation.base_terminal
        )
        for backend in ('native', 'wasm'):
            for name, case in cases.items():
                key = f'{suite}-{backend}-{name}'
                result = gate.run_case(name, case, backend, output / key)
                results[key] = result
                print(key, result, flush=True)
    authenticate(association)
    (output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    if any(result['status'] != 'PASS' for result in results.values()):
        raise RuntimeError('verification failed; see report.json')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('sources', 'build', 'verify', 'run'))
    parser.add_argument('--output', type=Path)
    parser.add_argument('--association', type=Path)
    args = parser.parse_args()
    cancellation = preflight()
    if args.action == 'sources':
        print(
            f'Tracked-source preflight passed: {len(sources())} inputs; no build artifacts required'
        )
        return
    if args.output is None:
        parser.error('--output is required')
    if args.action == 'verify' and args.association is None:
        parser.error('--association is required for verify')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with guard.session():
        association = args.association
        if args.action in ('build', 'run'):
            association = build(output)
        if args.action in ('verify', 'run'):
            verify(association, output, cancellation)
    print('PASS', output)


if __name__ == '__main__':
    main()
