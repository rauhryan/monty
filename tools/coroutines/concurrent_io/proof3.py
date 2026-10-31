"""Replay the fixed native-I/O cases at call acknowledgement and quiescent cuts."""

import argparse
import hashlib
import json
from pathlib import Path

from gate import ROOT, validate
from proof import REPO, invoke


def batch(replies=(), cancel=None):
    """Symbolic host plan, resolved only against observed call labels."""
    result = {'results': list(replies)}
    if cancel is not None:
        result['cancel'] = cancel
    return result


CASES = {
    'guest': (
        [batch([('B', 'beta')]), batch([('A', 'alpha')])],
        {'settled': ['B', 'A'], 'joined': [['A', 'alpha'], ['B', 'beta']]},
    ),
    'children_errors': (
        [batch([('B', 'ok')]), batch([('A', 'ok')]), batch([('C', 'beta')])],
        {'joined_error': 'child-error', 'trace': ['sibling-completed'], 'unconsumed_at_exit': ['other-error']},
    ),
    'children_drain': (
        [batch([('exit', 'ok')]), batch([('cleanup-A', 'ok')]), batch([('cleanup-B', 'ok')])],
        {'trace': ['cancel-A', 'cancel-B', 'drain-A', 'drain-B', 'parent-cleanup'], 'errors': []},
    ),
    'children_cancel': (
        [batch([('cancel', 'ok')]), batch([('B', 'beta')])],
        {'cancelled_join': ['cancelled', 'guest'], 'sibling_result': 'beta', 'completed_cancel': 'no-op', 'errors': []},
    ),
    'children_edges': (
        [batch([('exit', 'ok')]), batch([('cleanup', 'ok')])],
        {'trace': ['guest', 'unknown', 'unknown', 'unknown', 'foreign'], 'errors': ['cleanup-failure']},
    ),
    'children_protected': (
        [batch([('cancel', 'ok')]), batch([('cleanup', 'cleanup-ok')])],
        {'trace': ['cleanup-ok', 'guest'], 'errors': []},
    ),
    'host_cancel': (
        [
            batch([('A', 'must-not-deliver'), ('B', 'must-not-deliver')], ''),
            batch([('A', 'late'), ('cleanup-A', 'ok')], 'REPEAT'),
            batch([('B', 'late'), ('cleanup-B', 'ok')], 'REPEAT'),
        ],
        {
            'trace': [
                ['parent', ''],
                ['caught', 'A', ''],
                ['caught', 'B', ''],
                ['cleaned', 'A'],
                ['joined-cancelled', 'A', ''],
                ['cleaned', 'B'],
                ['joined-cancelled', 'B', ''],
            ],
            'errors': [],
        },
    ),
    'deferred_cleanup': (
        [batch([], 'FIRST'), batch([('second', 'second-ok')], 'REPEAT'), batch([('first', 'first-ok')], 'REPEAT')],
        {
            'trace': [
                ['caught', 'FIRST'],
                ['admission', 'child admission is closed'],
                ['cleanup', 'second'],
                ['reply', 'second-ok'],
                ['cleanup', 'first'],
                ['reply', 'first-ok'],
                ['scope', 'nested or cancelled child scope is not supported'],
            ],
            'errors': ['second-failure'],
        },
    ),
    'nested_cancel': (
        [batch([], 'FIRST'), batch([('leaf-cleanup', 'ok')], 'REPEAT'), batch([('owner-cleanup', 'ok')], 'REPEAT')],
        [
            ['parent-caught', 'FIRST'],
            ['leaf-caught', 'FIRST'],
            'leaf-cleaned',
            ['owner-caught', 'FIRST'],
            'owner-cleanup',
            'parent-cleanup',
        ],
    ),
    'drain_host_cancel': (
        [
            batch([('exit', 'ok')]),
            batch([('cleanup-A', 'ok')], 'FIRST'),
            batch([('cleanup-B', 'ok')], 'REPEAT'),
            batch([('parent-cleanup', 'ok')], 'REPEAT'),
        ],
        {
            'trace': ['drain-A', 'drain-B', ['caught', 'FIRST'], 'parent-cleanup'],
            'errors': ['scope-exit', 'scope-exit'],
        },
    ),
    'settlements': (
        [batch([('A', 'alpha'), ('A', 'wrong')]), batch([('A', 'late')]), batch([('B', 'beta')])],
        ['alpha', 'beta'],
    ),
}


def check(command, directory, name, steps, expected, cancel_reason=None):
    """Each external turn forks two fresh processes and disposes an independent copy."""
    directory.mkdir()
    evidence = []
    source = ROOT / f'{name}.py'
    snapshot = directory / 'cut-0.bin'
    view = invoke(command, ['start-step', source, snapshot], evidence)
    views, batches, cuts, probes = [view], [], [], []
    ids = {}
    turn = 0
    while 'complete' not in view:
        index = len(batches)
        if 'call' in view:
            ids[view['call']['label']] = view['call']['id']
            reply = {'ack': True}
        else:
            assert turn < len(steps), (name, view, turn)
            plan = steps[turn]
            reply = plan | {'results': [{'id': ids[label], 'value': value} for label, value in plan['results']]}
            turn += 1
        original = snapshot.read_bytes()
        assert invoke(command, ['dispose', snapshot], evidence) == {'disposed': True}
        # Invalid full batches include a valid first reply to detect partial mutation.
        if name == 'settlements' and 'pending' in view and not probes:
            for kind, bad in [('unknown', {'id': 4294967295, 'value': 'bad'}), ('invalid', {'id': -1, 'value': 'bad'})]:
                target = directory / f'rejected-{kind}.bin'
                rejected = invoke(
                    command,
                    ['reject', snapshot, json.dumps([{'id': ids['A'], 'value': 'corrupt'}, bad]), target],
                    evidence,
                )
                assert rejected.get('rejected'), rejected
                probe_next = directory / f'rejected-{kind}-next.bin'
                actual = invoke(command, ['resume-step', target, json.dumps(reply), probe_next], evidence)
                probes.append({'kind': kind, 'error': rejected, 'next': actual})
        target = directory / f'cut-{index + 1}.bin'
        branch = directory / f'branch-{index}.bin'
        actual = invoke(command, ['resume-step', snapshot, json.dumps(reply), target], evidence)
        duplicate = invoke(command, ['resume-step', snapshot, json.dumps(reply), branch], evidence)
        assert actual == duplicate and snapshot.read_bytes() == original
        if probes and index == 2:
            assert all(probe['next'] == actual for probe in probes)
        if 'complete' not in actual:
            assert invoke(command, ['dispose', branch], evidence) == {'disposed': True}
        cuts.append(
            {
                'kind': 'FunctionCall' if 'call' in view else 'ResolveFutures',
                'sha256': hashlib.sha256(original).hexdigest(),
                'unchanged': snapshot.read_bytes() == original,
                'fresh_equal': actual == duplicate,
                'disposed': True,
            }
        )
        batches.append(reply)
        views.append(actual)
        snapshot, view = target, actual
    assert turn == len(steps), (name, turn)
    terminal = view['complete']
    if cancel_reason is not None or name in ('host_cancel', 'deferred_cleanup', 'nested_cancel', 'drain_host_cancel'):
        assert terminal['status'] == 'cancelled'
        assert terminal['reason'] == (
            cancel_reason if cancel_reason is not None else ('' if name == 'host_cancel' else 'FIRST')
        )
        assert terminal['error'] is None and terminal['value_export_error'] is None
        payload = json.loads(terminal['value'])
    else:
        payload = terminal
    assert payload == expected, (name, payload, expected)
    batches_file = directory / 'batches.json'
    batches_file.write_text(json.dumps(batches))
    continuous = invoke(command, ['run-step', source, batches_file], evidence)
    assert continuous == views
    result = {
        'views': views,
        'payload': payload,
        'terminal': terminal,
        'cuts': cuts,
        'continuous_equal': continuous == views,
        'probes': probes,
        'commands': evidence,
    }
    (directory / 'evidence.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--association', type=Path)
    parser.add_argument('--backend', choices=['native', 'wasm', 'both'], default='both')
    parser.add_argument('--ownership-report', type=Path)
    args = parser.parse_args()
    if args.association:
        association = json.loads(args.association.read_text())
        for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
            for name, digest in association[group].items():
                assert hashlib.sha256((REPO / name).read_bytes()).hexdigest() == digest, name
    args.output.mkdir(parents=True, exist_ok=False)
    backends = ['native', 'wasm'] if args.backend == 'both' else [args.backend]
    results = {}
    for backend in backends:
        directory = args.output / backend
        directory.mkdir()
        command = (
            [str(REPO / 'playground/coroutine-baseline/target/debug/concurrent_io')]
            if backend == 'native'
            else ['node', str(ROOT / 'wasm.mjs')]
        )
        results[backend] = {name: check(command, directory / name, name, *spec) for name, spec in CASES.items()}
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    if args.backend != 'both' or not args.ownership_report:
        print('Exploratory execution passed; no full gate claim.')
        return
    ownership = json.loads(args.ownership_report.read_text())
    log = args.ownership_report.with_suffix('.log').read_text()
    assert ownership['exit'] == 0
    consumed = 'owned-child wake consumed=false; delivered consumed=true; scope root removed' in log
    assert consumed
    for name in CASES:
        assert results['native'][name]['views'] == results['wasm'][name]['views']
    data = results['native']
    payload = {name: case['payload'] for name, case in data.items()}
    cancelled = payload['host_cancel']['trace']
    cleanup = payload['deferred_cleanup']
    settlements = data['settlements']
    all_cases = [case for cases in results.values() for case in cases.values()]
    observations = {
        '1': {'calls': [v['call']['label'] for v in data['guest']['views'] if 'call' in v], **payload['guest']},
        '2': {
            'joined_error': payload['children_errors']['joined_error'],
            'consumed_after_delivery': consumed,
            'unconsumed_at_exit': payload['children_errors']['unconsumed_at_exit'],
            'sibling_completed': payload['children_errors']['trace'] == ['sibling-completed'],
        },
        '3': {
            'children_cancelled': [row[1] for row in cancelled if row[0] == 'joined-cancelled'],
            'caught': [row[1] for row in cancelled if row[0] == 'caught'],
            'reason': data['host_cancel']['terminal']['reason'],
            'status': data['host_cancel']['terminal']['status'],
        },
        '4': {'trace': payload['children_drain']['trace']},
        '5': {key: payload['children_cancel'][key] for key in ('cancelled_join', 'sibling_result', 'completed_cancel')},
        '6': {
            'cleanup': [row[1] for row in cleanup['trace'] if row[0] == 'cleanup'],
            'replies': [row[1] for row in cleanup['trace'] if row[0] == 'reply'],
            'errors': cleanup['errors'],
            'reason': data['deferred_cleanup']['terminal']['reason'],
            'status': data['deferred_cleanup']['terminal']['status'],
            'admission': 'closed' if ['admission', 'child admission is closed'] in cleanup['trace'] else 'open',
        },
        '7': {
            'first_result': settlements['payload'][0],
            'other_task': settlements['payload'][1],
            'duplicate': 'ignored' if settlements['payload'][0] == 'alpha' else 'delivered',
            'late': 'ignored' if settlements['views'][-2] == settlements['views'][-3] else 'changed',
            'unknown': 'explicit-error' if settlements['probes'][0]['error']['rejected'] else 'accepted',
            'invalid': 'explicit-error' if settlements['probes'][1]['error']['rejected'] else 'accepted',
            'resurrected': any(
                'call' in v and v['call']['label'] in ('A', 'B') for v in data['host_cancel']['views'][3:]
            )
            or len([row for row in cancelled if row[0] == 'caught']) != 2,
        },
        '8': {
            'native': len(results['native']) == len(CASES),
            'wasm': len(results['wasm']) == len(CASES),
            'uninterrupted': all(c['continuous_equal'] for c in all_cases),
            'fresh_restore_every_boundary': all(cut['fresh_equal'] for c in all_cases for cut in c['cuts']),
            'independent_disposal': all(cut['disposed'] for c in all_cases for cut in c['cuts']),
            'checkpoint_unchanged': all(cut['unchanged'] for c in all_cases for cut in c['cuts']),
        },
    }
    (args.output / 'observations.json').write_text(json.dumps(observations, indent=2) + '\n')
    gate = validate(observations)
    (args.output / 'gate.json').write_text(json.dumps(gate, indent=2) + '\n')
    print(json.dumps(gate, indent=2))


if __name__ == '__main__':
    main()
