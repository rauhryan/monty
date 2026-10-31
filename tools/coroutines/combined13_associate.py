"""Bind library/test repair to unchanged runtime artifacts, preserving old receipts."""

import hashlib
import json
import subprocess
from pathlib import Path
from trace import digest, finite_dumps

from scope9 import runtime_identity
from verify import REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent


def main():
    previous_path = ROOT / 'combined12_artifacts.json'
    previous = json.loads(previous_path.read_text())
    assert runtime_identity() == previous['runtime_sha256']
    path = Path('crates/monty/tests/generator_runtime.rs')
    raw = path.read_bytes()
    prefix = raw[: raw.index(b'\n/// R12-1: new contexts and rejected input owners')]
    assert hashlib.sha256(prefix).hexdigest() == previous['test_only_delta']['current_file_sha256']
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD']).strip()
    diff = subprocess.check_output(
        ['git', 'diff', '--binary', 'HEAD', '--', 'crates', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']
    )
    names = subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z', 'crates'])
    added = {n: digest(REPO / n) for n in names.decode().split('\0') if n}
    added[str(path)] = hashlib.sha256(prefix).hexdigest()
    normalized = hashlib.sha256(head + b'\n' + diff + b'\n' + finite_dumps(added, sort_keys=True).encode()).hexdigest()
    assert normalized == previous['source_sha256']
    current = source_identity()
    association = (
        'Runtime unchanged from combined12; no runtime rebuild. '
        'Only appended generator_runtime.rs tests change the interpreter/test source digest. '
        'Normalizing that preserved prefix reproduces combined12. Updated library and test inputs are bound separately.'
    )
    for backend in ('native', 'wasm'):
        assert {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)} == previous[backend]['artifacts']
        previous[backend]['source_sha256'] = current
        previous[backend]['association'] = association
    previous['source_sha256'] = current
    previous['association'] = association
    previous['previous_receipt_sha256'] = digest(previous_path)
    previous['test_only_delta'] = {
        'file': str(path),
        'preserved_bytes': len(prefix),
        'preserved_sha256': added[str(path)],
        'normalized_full_source_sha256': normalized,
        'current_file_sha256': digest(path),
    }
    previous['combined_inputs'] = {
        str(p): digest(p)
        for p in [*ROOT.glob('combined11*.py'), *ROOT.glob('combined12*.py'), *ROOT.glob('combined13*.py')]
    }
    previous['fixed_oracles'] = {
        str(p): digest(p)
        for name in ('expected.json', 'cases.json', 'scope7_expected.json', 'scope7_scripts.json')
        if (p := ROOT / name).exists()
    }
    with (ROOT / 'combined13_artifacts.json').open('x') as output:
        output.write(finite_dumps(previous, indent=2) + '\n')
    print(current)


if __name__ == '__main__':
    main()
