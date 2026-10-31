"""Literal additive gather lifetime regressions; reuse fresh-process cut replay."""

import argparse
import hashlib
import json
from pathlib import Path

from proof import REPO
from proof3 import ROOT, batch, check

CASES = {
    'gather_host': (
        [
            batch([], 'FIRST'),
            batch([('cleanup-B', 'ok')], 'REPEAT'),
            batch([('cleanup-A', 'ok')], 'REPEAT'),
            batch([('parent-cleanup', 'ok')], 'REPEAT'),
        ],
        {
            'trace': [['parent', 'FIRST'], ['A', 'FIRST'], ['B', 'FIRST'], 'cleaned-B', 'cleaned-A', 'parent-cleanup'],
            'errors': ['cleanup-A-failure'],
        },
        'FIRST',
    ),
    'gather_first_error': (
        [
            batch([('A', 'ok')]),
            batch([('cleanup-A', 'ok')]),
            batch([('continue', 'ok')]),
            batch([('cleanup-B', 'ok')]),
            batch([('parent-cleanup', 'ok')]),
        ],
        {'trace': ['cleaned-A', 'first-error', 'cleaned-B', 'parent-cleanup'], 'errors': ['cleanup-B-failure']},
        None,
    ),
    'gather_protected': (
        [
            batch([('exit', 'ok')]),
            batch([('cleanup-B', 'beta')], 'FIRST'),
            batch([('cleanup-A', 'alpha')], 'REPEAT'),
            batch([('parent-cleanup', 'ok')], 'REPEAT'),
        ],
        {
            'trace': [['B', 'beta'], ['A', 'alpha'], ['parent', 'FIRST'], 'parent-cleanup'],
            'errors': ['protected-B-failure'],
        },
        'FIRST',
    ),
    'gather_guest': (
        [
            batch([('cancel', 'ok')]),
            batch([('cleanup-B', 'ok')]),
            batch([('cleanup-A', 'ok')]),
            batch([('C', 'independent')]),
            batch([('parent-cleanup', 'ok')]),
        ],
        {
            'trace': [
                ['parent', 'guest'],
                ['A', 'guest'],
                ['B', 'guest'],
                'cleaned-B',
                'cleaned-A',
                ['unrelated', 'independent'],
                'parent-cleanup',
            ],
            'errors': ['cleanup-A-failure'],
        },
        None,
    ),
    'gather_external': (
        [batch([('mode', 'cancel')]), batch([('exit', 'ok')]), batch([('parent-cleanup', 'ok')])],
        {'trace': ['guest', 'parent-cleanup'], 'errors': []},
        None,
    ),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--association', type=Path)
    parser.add_argument('--backend', choices=['native', 'wasm', 'both'], default='both')
    args = parser.parse_args()
    if args.association:
        association = json.loads(args.association.read_text())
        for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
            for name, digest in association[group].items():
                assert hashlib.sha256((REPO / name).read_bytes()).hexdigest() == digest, name
    args.output.mkdir(parents=True)
    report = {}
    for backend in ['native', 'wasm'] if args.backend == 'both' else [args.backend]:
        directory = args.output / backend
        directory.mkdir()
        command = (
            [str(REPO / 'playground/coroutine-baseline/target/debug/concurrent_io')]
            if backend == 'native'
            else ['node', str(ROOT / 'wasm.mjs')]
        )
        cases = {name: check(command, directory / name, name, *spec) for name, spec in CASES.items()}
        cases['external_success'] = check(
            command,
            directory / 'external_success',
            'gather_external',
            [
                batch([('mode', 'success')]),
                batch([('exit', 'ok')]),
                batch([('direct-B', 'beta')]),
                batch([('direct-A', 'alpha')]),
                batch([('parent-cleanup', 'ok')]),
            ],
            {'trace': [['alpha', ['beta']], 'parent-cleanup'], 'errors': []},
        )
        cases['external_host'] = check(
            command,
            directory / 'external_host',
            'gather_external',
            [
                batch([('mode', 'host')]),
                batch([('direct-A', 'revoked'), ('direct-B', 'revoked')], 'FIRST'),
                batch([('direct-A', 'late'), ('parent-cleanup', 'ok')], 'REPEAT'),
            ],
            {'trace': [['host', 'FIRST'], 'FIRST', 'parent-cleanup'], 'errors': []},
            'FIRST',
        )
        # First-error is visible while B remains in its body: no sibling failfast.
        views = cases['gather_first_error']['views']
        b_id = next(v['call']['id'] for v in views if v.get('call', {}).get('label') == 'B')
        continuation = next(i for i, v in enumerate(views) if v.get('call', {}).get('label') == 'continue')
        assert b_id in views[continuation + 1]['pending']
        assert not any(v.get('call', {}).get('label') == 'cleanup-B' for v in views[:continuation])
        # Both cleanups must finish before the parent calls its cleanup operation.
        for name in ('gather_host', 'gather_first_error', 'gather_guest', 'gather_protected'):
            case = cases[name]
            assert next(v for v in case['views'] if v.get('call', {}).get('label') == 'parent-cleanup')
            final_pending = [v['pending'] for v in case['views'] if 'pending' in v][-1]
            assert len(final_pending) == 1
        report[backend] = cases
    if args.backend == 'both':
        assert all(report['native'][name]['views'] == report['wasm'][name]['views'] for name in report['native'])
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('Gather lifetime supplemental replay passed:', ', '.join(report))


if __name__ == '__main__':
    main()
