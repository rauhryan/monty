"""Aggregate long-run evidence; rotating snapshots, restored at every external cut."""

import argparse
import json
import types
from pathlib import Path
from trace import digest, event_from, execute, finite_dumps

from coordinator1 import proof as p
from verify import BASELINE

BRIDGE = """
import json
config = json.loads(exchange(json.dumps({'configure': 'stress'})))
state = new_driver(config['mode'], False)
pump(state, [])
while state['tasks']['T0']['result'] is None:
    messages = json.loads(exchange(json.dumps(measurement(state))))
    pump(state, messages)
json.dumps(measurement(state))
"""


def source():
    return '\n'.join((p.ROOT / n).read_text() for n in (*p.FILES[:3], 'stress_workload.py'))


def run(backend, mode, output):
    ns = {}
    exec(source().replace(p.DIRECTIVE, ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda b: b.__await__()
    state = ns['new_driver'](mode, False)
    directory = output / backend / mode
    directory.mkdir(parents=True)
    guest = directory / 'guest.py'
    guest.write_text(source() + '\ndef adapt(body):\n    return body\n' + BRIDGE)
    command = (
        [str(BASELINE / 'target/debug/coroutine-baseline')]
        if backend == 'native'
        else ['node', str(BASELINE / 'wasm.mjs')]
    )

    def invoke(args):
        return event_from(execute(command + args, 120))

    snapshot = directory / 'current.bin'
    target = directory / 'next.bin'
    assert invoke(['start', str(guest), str(snapshot)]) == {'effect': {'configure': 'stress'}}
    message = {'mode': mode}
    ns['pump'](state, [])
    measurements = []
    serial = 0
    for cut in range(100):
        expected = p.detached(ns['measurement'](state))
        assert invoke(['dispose', str(snapshot)]) == {'disposed': True}
        actual = invoke(['resume', str(snapshot), finite_dumps(message), str(target)])
        key = 'complete' if expected['terminal'] is not None else 'effect'
        assert actual == {key: expected}, (backend, mode, cut, actual, expected)
        measurements.append(
            {
                'cut': cut,
                'cycles': expected['cycles'],
                'tasks': len(expected['tasks']),
                'scopes': len(expected['scopes']),
                'events': len(expected['events']),
                'created': expected['created_tasks'],
                'reclaimed': expected['reclaimed_tasks'],
                'snapshot_bytes': target.stat().st_size if key == 'effect' else None,
            }
        )
        if key == 'complete':
            if mode == 'success':
                assert expected['terminal']['value'] == 1499 * 1500 // 2
                assert expected['created_tasks'] == 3001
            else:
                assert expected['terminal']['kind'] == 'cancelled'
                assert expected['terminal']['cancel'] == ''
                assert expected['terminal']['children'][0]['failures'] == ['audit-stress']
                payloads = [e[1]['payload'] for e in expected['events'] if e[0] == 'pause']
                assert payloads[-3:] == [['audit', 'worker'], ['close', 'worker'], ['close', 'parent']]
            break
        target.replace(snapshot)
        if cut in (0, 10, 20) or any(x['payload'][0] == 'audit' for x in expected['pending']):
            (directory / f'representative-{cut}.bin').write_bytes(snapshot.read_bytes())
        message = []
        if cut == 10:
            serial += 1
            late = {
                'run': 'C1',
                'event': serial,
                'kind': 'reply',
                'operation': 'B0',
                'outcome': 'value',
                'value': 'LATE',
            }
            message = [late]
            ns['pump'](state, message)
            after = ns['measurement'](state)
            assert after['pending'] == expected['pending']
            assert after['cycles'] == expected['cycles']
            assert after['events'][-1] == ['reject', late]
            continue
        for op in expected['pending']:
            serial += 1
            if op['payload'][0] == 'hold':
                message.append({'run': 'C1', 'event': serial, 'kind': 'cancel', 'reason': ''})
            else:
                failure = op['payload'][0] == 'audit'
                message.append(
                    {
                        'run': 'C1',
                        'event': serial,
                        'kind': 'reply',
                        'operation': op['boundary'],
                        'outcome': 'error' if failure else 'value',
                        'value': 'audit-stress' if failure else None,
                    }
                )
        # Every turn also rejects a permanently retired operation, without a tombstone.
        serial += 1
        message.append(
            {'run': 'C1', 'event': serial, 'kind': 'reply', 'operation': 'B0', 'outcome': 'value', 'value': None}
        )
        ns['pump'](state, message)
    else:
        raise AssertionError('stress did not complete')
    sizes = [m['snapshot_bytes'] for m in measurements if m['cycles'] >= 100 and m['snapshot_bytes'] is not None]
    assert max(sizes) - min(sizes) < 32768, sizes
    return {
        'status': 'PASS',
        'isolated_retired_reply_rejected_at_cycle': 550,
        'measurements': measurements,
        'post_warmup_snapshot_min': min(sizes),
        'post_warmup_snapshot_max': max(sizes),
        'terminal': expected['terminal'],
        'guest_sha256': digest(guest),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    report = {'identity': p.identity(), 'runs': {}, 'status': 'FAIL'}
    try:
        for backend in ('native', 'wasm'):
            for mode in ('success', 'cancel'):
                report['runs'][backend + '/' + mode] = run(backend, mode, args.output)
                print(backend, mode, 'PASS', flush=True)
        report['status'] = 'PASS'
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
