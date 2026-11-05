"""Build and replay native/Wasm concurrency regressions."""

import argparse
import json
import shutil
from pathlib import Path

import guard
import replay

ROOT = Path(__file__).resolve().parent
REPO = guard.REPO
FIXTURES = REPO / 'crates/monty/tests/fixtures/concurrency'
NATIVE = guard.TARGET / 'debug/examples/concurrency_transport'
digest = replay.digest


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
    paths.extend(files_under(FIXTURES))
    paths.extend(
        [
            REPO / 'crates/monty/tests/support/concurrency_transport.rs',
            REPO / 'crates/monty-js/scripts/build-component.mjs',
            REPO / 'crates/monty-js/package.json',
            REPO / 'crates/monty-js/package-lock.json',
        ]
    )
    return {str(p.relative_to(REPO)): digest(p) for p in sorted(set(paths))}


def artifacts():
    paths = [NATIVE, guard.TARGET / 'wasm32-wasip1/release/monty_wasm_runtime.wasm']
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
    """Preserve the frozen public-async oracle and source hashes across relocation."""
    frozen = FIXTURES / 'public_async'
    for name, expected in json.loads(
        (frozen / 'contract.sha256.json').read_text()
    ).items():
        path = frozen / (name + '.txt' if name.endswith('.py') else name)
        replay.check(digest(path) == expected, 'frozen input changed: ' + name)
    cases = {
        'native_async/' + name: case
        | {'source': 'public_async/cases/' + name + '.py.txt'}
        for name, case in json.loads((frozen / 'contract.json').read_text())[
            'cases'
        ].items()
    }
    cases.update(replay.strict_json((ROOT / 'cases.json').read_text()))
    replay.check(len(cases) == 14, 'unexpected case inventory')
    for name, case in cases.items():
        replay.check(
            len(case['steps']) == len(case['batches']) > 0,
            'invalid host script: ' + name,
        )
        source = (FIXTURES / case['source']).read_text()
        replay.check(
            '__monty_host__' not in source and '._native' not in source,
            'experimental guest entry',
        )
    replay.self_check()
    return cases


def build(output):
    before = sources()
    steps = {
        'native': [
            'cargo',
            'build',
            '--locked',
            '-p',
            'monty',
            '--example',
            'concurrency_transport',
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


def verify(association, output, cases):
    authenticate(association)
    results = {}
    for backend in ('native', 'wasm'):
        command = (
            [str(NATIVE)] if backend == 'native' else ['node', str(ROOT / 'wasm.mjs')]
        )
        for name, case in cases.items():
            key = backend + '-' + name.replace('/', '-')
            result = replay.run_case(
                case, command, FIXTURES / case['source'], output / key
            )
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
    cases = preflight()
    if args.action == 'sources':
        print(
            f'Source preflight passed: {len(sources())} inputs; no build artifacts required'
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
            verify(association, output, cases)
    print('PASS', output)


if __name__ == '__main__':
    main()
