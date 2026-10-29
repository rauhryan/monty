"""Execute child joins/cancellation/drain on native and actual Wasm, at fresh cuts."""

import argparse
import hashlib
import json
from pathlib import Path

from gate import ROOT, validate
from proof import REPO, invoke


def check_case(command, directory, source, steps, expected):
    """Keep every transport turn, independent restored branch and checkpoint hash."""
    directory.mkdir()
    evidence = []
    snapshot = directory / 'cut-0.bin'
    view = invoke(command, ['start', source, snapshot], evidence)
    views = [view]
    ids = {call['label']: call['id'] for call in view['calls']}
    batches = []
    for index, replies in enumerate(steps):
        original = snapshot.read_bytes()
        assert invoke(command, ['dispose', snapshot], evidence) == {'disposed': True}
        batch = [{'id': ids[label], 'value': value} for label, value in replies]
        batches.append(batch)
        target = directory / f'cut-{index + 1}.bin'
        view = invoke(
            command, ['resume', snapshot, json.dumps(batch), target], evidence
        )
        branch = directory / f'branch-{index}.bin'
        duplicate = invoke(
            command, ['resume', snapshot, json.dumps(batch), branch], evidence
        )
        assert view == duplicate and snapshot.read_bytes() == original
        evidence.append(
            {
                'checkpoint_sha256': hashlib.sha256(original).hexdigest(),
                'unchanged': snapshot.read_bytes() == original,
            }
        )
        if 'pending' in view:
            assert invoke(command, ['dispose', branch], evidence) == {'disposed': True}
            ids.update({call['label']: call['id'] for call in view['calls']})
        views.append(view)
        snapshot = target
    assert views[-1] == {'complete': expected}, views
    batches_file = directory / 'batches.json'
    batches_file.write_text(json.dumps(batches))
    continuous = invoke(command, ['run', source, batches_file], evidence)
    assert continuous == views
    (directory / 'evidence.json').write_text(
        json.dumps({'views': views, 'commands': evidence}, indent=2) + '\n'
    )
    return views[-1]['complete']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--association', type=Path, required=True)
    parser.add_argument('--ownership-report', type=Path, required=True)
    parser.add_argument('--round1-observations', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    association = json.loads(args.association.read_text())
    for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
        for name, digest in association[group].items():
            assert hashlib.sha256((REPO / name).read_bytes()).hexdigest() == digest, (
                name
            )
    ownership = json.loads(args.ownership_report.read_text())
    log = args.ownership_report.with_suffix('.log').read_text()
    assert ownership['exit'] == 0
    assert (
        'owned-child wake consumed=false; delivered consumed=true; scope root removed'
        in log
    )
    assert 'ownership_concurrent2_bounded_records ... ok' in log
    args.output.mkdir(parents=True, exist_ok=False)
    expected = json.loads((ROOT / 'expected.json').read_text())
    results = {}
    for backend in ('native', 'wasm'):
        directory = args.output / backend
        directory.mkdir()
        command = (
            [str(REPO / 'playground/coroutine-baseline/target/debug/concurrent_io')]
            if backend == 'native'
            else ['node', str(ROOT / 'wasm.mjs')]
        )
        errors = check_case(
            command,
            directory / 'errors',
            ROOT / 'children_errors.py',
            [[('B', 'ok')], [('A', 'ok')], [('C', 'beta')]],
            {
                'joined_error': 'child-error',
                'trace': ['sibling-completed'],
                'unconsumed_at_exit': ['other-error'],
            },
        )
        drain = check_case(
            command,
            directory / 'drain',
            ROOT / 'children_drain.py',
            [[('exit', 'ok')], [('cleanup-A', 'ok')], [('cleanup-B', 'ok')]],
            {'trace': expected['4']['trace'], 'errors': []},
        )
        cancel = check_case(
            command,
            directory / 'cancel',
            ROOT / 'children_cancel.py',
            [[('cancel', 'ok')], [('B', 'beta')]],
            expected['5'] | {'errors': []},
        )
        check_case(
            command,
            directory / 'edges',
            ROOT / 'children_edges.py',
            [[('exit', 'ok')], [('cleanup', 'ok')]],
            {
                'trace': ['guest', 'unknown', 'unknown', 'unknown', 'foreign'],
                'errors': ['cleanup-failure'],
            },
        )
        check_case(
            command,
            directory / 'protected',
            ROOT / 'children_protected.py',
            [[('cancel', 'ok')], [('cleanup', 'cleanup-ok')]],
            {'trace': ['cleanup-ok', 'guest'], 'errors': []},
        )
        results[backend] = {
            '2': {
                'joined_error': errors['joined_error'],
                'consumed_after_delivery': 'delivered consumed=true' in log
                and 'wake consumed=false' in log,
                'unconsumed_at_exit': errors['unconsumed_at_exit'],
                'sibling_completed': errors['trace'] == ['sibling-completed'],
            },
            '4': {'trace': drain['trace']},
            '5': {key: cancel[key] for key in expected['5']},
        }
    assert results['native'] == results['wasm']
    observations = json.loads(args.round1_observations.read_text()) | results['native']
    (args.output / 'observations.json').write_text(
        json.dumps(observations, indent=2) + '\n'
    )
    report = {
        'association_sha256': hashlib.sha256(args.association.read_bytes()).hexdigest(),
        'ownership_receipt_sha256': hashlib.sha256(
            args.ownership_report.read_bytes()
        ).hexdigest(),
        'gate': validate(observations),
        'backends': results,
        'recovery_scope': 'requirements 1, 2, 4, 5; quiescent external cuts only',
    }
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
