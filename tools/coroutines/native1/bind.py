"""Associate a real guarded native/Wasm build with immutable current candidate inputs."""

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def digest(path):
    """Hash exact consumed bytes."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inputs():
    """Use the verifier's runtime scope including untracked nonignored Rust inputs."""
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=REPO, text=True
    ).splitlines()
    runtime = {
        n: digest(REPO / n)
        for n in sorted(set(names))
        if (n.startswith('crates/') and ('/src/' in n or n.endswith('Cargo.toml')))
        or n in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
    }
    extra = {str(p.relative_to(REPO)): digest(p) for p in REPO.glob('crates/**/build.rs')}
    extra.update({str(p.relative_to(REPO)): digest(p) for p in (REPO / '.cargo').glob('*') if p.is_file()})
    loaded = [p for pattern in ('crates/**/*.wit', 'crates/**/*.proto') for p in REPO.glob(pattern)]
    loaded += list(ROOT.glob('*')) + list((REPO / 'playground/coroutine-baseline/native').glob('Cargo.*'))
    loaded += [
        REPO / 'playground/coroutine-baseline/native/src/main.rs',
        REPO / 'crates/monty-js/scripts/build-component.mjs',
        REPO / 'tools/coroutines/conformance/workloads.py',
        REPO / 'tools/coroutines/coordinator1/guarded_checks.py',
        REPO / 'playground/coroutine-baseline/build.py',
    ]
    files = {str(p.relative_to(REPO)): digest(p) for p in loaded if p.is_file()}
    return {'runtime': runtime, 'build_inputs': extra, 'files': files}


def main():
    """Build through the existing lock, reject source drift, never rewrite old receipts."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    before = inputs()
    receipts = {}
    for action in ('native', 'wasm', 'component'):
        receipt = args.output / (action + '.json')
        subprocess.run(
            [sys.executable, str(ROOT.parent / 'coordinator1/guarded_checks.py'), action, '--report', str(receipt)],
            cwd=REPO,
            check=True,
        )
        receipts[str(receipt.relative_to(REPO) if receipt.is_absolute() else receipt)] = digest(receipt)
    assert before == inputs(), 'candidate source changed during build'
    files = before['files'] | receipts
    artifacts = [
        REPO / 'playground/coroutine-baseline/target/debug/native1',
        REPO / 'playground/coroutine-baseline/target/wasm32-wasip1/release/monty_wasm_runtime.wasm',
    ]
    artifacts += list((REPO / 'crates/monty-js/dist/worker/component').glob('*.wasm'))
    artifacts += list((REPO / 'crates/monty-js/dist/worker/component').glob('*.js'))
    artifacts += list((REPO / 'crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist').rglob('*.js'))
    files.update({str(p.relative_to(REPO)): digest(p) for p in artifacts})
    for executable in (Path(sys.executable).resolve(), Path(shutil.which('node')).resolve()):
        files[str(executable)] = digest(executable)
    association = args.output / 'build-association.json'
    association.write_text(
        json.dumps(
            before | {'receipts': receipts, 'artifacts': {str(p.relative_to(REPO)): digest(p) for p in artifacts}},
            indent=2,
        )
        + '\n'
    )
    key = str(association.relative_to(REPO) if association.is_absolute() else association)
    files[key] = digest(association)
    binding = {
        'version': 1,
        'command': [sys.executable, str(ROOT / 'adapter.py')],
        'files': files,
        'build_receipt': key,
        'runtime_sha256': hashlib.sha256(json.dumps(before['runtime'], sort_keys=True).encode()).hexdigest(),
        'build_inputs': before['build_inputs'],
    }
    (args.output / 'binding.json').write_text(json.dumps(binding, indent=2) + '\n')
    print(args.output / 'binding.json')


if __name__ == '__main__':
    main()
