"""Literal queue expectations frozen before implementation; reuse external cut replay."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'concurrent_io'))
import proof3
from proof3 import batch

CASES = {
    'fifo': ([batch([('full', 'ok')]), batch([('contents', 'ok')])],
             {'values': ['seed', 'A', 'B', [1, 2, 3]], 'puts': ['A', 'B']}),
    'waiters': ([batch([('empty', 'ok')])], [['B', 'guest'], ['A', 'one'], ['C', 'two']]),
    'cancel': ([batch([], 'FIRST'), batch([('cleanup-get', 'get-ok')], 'REPEAT'),
                batch([('cleanup-put', 'put-ok')], 'REPEAT'), batch([('parent-cleanup', 'ok')], 'REPEAT')],
               {'trace': [['parent', 'FIRST'], ['put', 'FIRST'], ['get', 'FIRST'],
                          ['get', 'get-ok'], ['put', 'put-ok'], 'parent-cleanup'], 'errors': []}, 'FIRST'),
}


def main():
    """Require artifact association and save every native/Wasm execution receipt."""
    parser = argparse.ArgumentParser()
    parser.add_argument('--association', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    association = json.loads(args.association.read_text())
    for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
        for name, digest in association[group].items():
            assert hashlib.sha256((proof3.REPO / name).read_bytes()).hexdigest() == digest, name
    args.output.mkdir(parents=True, exist_ok=False)
    proof3.ROOT = ROOT
    report = {}
    for backend in ('native', 'wasm'):
        directory = args.output / backend
        directory.mkdir()
        command = ([str(proof3.REPO / 'playground/coroutine-baseline/target/debug/concurrent_io')]
                   if backend == 'native' else ['node', str(ROOT.parent / 'concurrent_io/wasm.mjs')])
        report[backend] = {name: proof3.check(command, directory / name, name, *spec)
                           for name, spec in CASES.items()}
    assert all(report['native'][n]['views'] == report['wasm'][n]['views'] for n in CASES)
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('Queue external gate passed: native, actualWasm')


if __name__ == '__main__':
    main()
