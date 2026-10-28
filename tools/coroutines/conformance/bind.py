"""Intentional binding proposal ONLY; never changes fixtures or historic receipts."""

import argparse
import hashlib
import json
from pathlib import Path

from adapter import REPO, ROOT

RECEIPT = 'playground/coroutine-implementation/bounded-repair2-association1.json'
RECEIPT_SHA = '1443c36e6e95c92c00ccfe825fcb9ea9da980fab841ae4ef397f686264ce3a9c'


def digest(path):
    """Hash exact source bytes, not HEAD contents."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    """Write a new reviewed-version proposal to an exclusive output path."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--reason', required=True)
    args = parser.parse_args()
    if digest(REPO / RECEIPT) != RECEIPT_SHA:
        raise ValueError('accepted receipt changed')
    receipt = json.loads((REPO / RECEIPT).read_text())
    files = {RECEIPT: RECEIPT_SHA}
    for name in ('bindings.py', 'kernel.py', 'coordinator.py'):
        expected = receipt['runtime_and_adapter_association']['inputs'][name]
        if digest(ROOT / 'reference' / name) != expected:
            raise ValueError('reference is not accepted current source: ' + name)
    for backend in receipt['runtime_and_adapter_association']['artifacts'].values():
        for path, expected in backend.items():
            if digest(REPO / path) != expected:
                raise ValueError('artifact mismatch: ' + path)
            files[path] = expected
    # Source associated now in addition to, not substituted for, the historic receipt.
    inputs = list(ROOT.glob('*.py')) + list((ROOT / 'reference').glob('*.py'))
    inputs += list((REPO / 'playground/coroutine-baseline/native').glob('src/*.rs'))
    inputs += list((REPO / 'playground/coroutine-baseline/native').glob('Cargo.*'))
    # Actual Wasm host JS dependencies, including untracked/ignored loaded source.
    inputs += list((REPO / 'crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist').rglob('*.js'))
    for path in inputs:
        files[str(path.relative_to(REPO))] = digest(path)
    build = {str(p.relative_to(REPO)): digest(p) for p in REPO.glob('crates/**/build.rs')}
    build.update({str(p.relative_to(REPO)): digest(p) for p in (REPO / '.cargo').glob('*') if p.is_file()})
    binding = {
        'version': 1,
        'reason': args.reason,
        'reference_head_context': receipt['head'],
        'reference_receipt': RECEIPT,
        'reference_is_uncommitted': True,
        'runtime_sha256': receipt['runtime_and_adapter_association']['runtime'],
        'files': files,
        'build_inputs': build,
        'cache_paths': list(receipt['disk']['targets']),
    }
    with args.output.open('x') as output:
        json.dump(binding, output, indent=2)
        output.write('\n')


if __name__ == '__main__':
    main()
