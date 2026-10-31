"""Replay the original policy matrix with reclamation and its ordered event protocol."""

import argparse
import json
import types
from pathlib import Path

from coordinator1 import proof as p


def bounded_case(original, guest):
    ns = {}
    exec(guest.split('\nimport json\nconfig =')[0].replace(p.DIRECTIVE, ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda b: b.__await__()
    mode = original['inputs'][0]['mode']
    state = ns['new_driver'](mode, False)
    ns['turn'](state, [], 4)
    inputs = [{'mode': mode}]
    views = [p.detached(ns['observe'](state))]
    ids = {}
    for batch, expected in zip(original['inputs'][1:], original['views'][1:], strict=True):
        messages = []
        for message in batch:
            old = message['event']
            if old not in ids:
                ids[old] = len(ids) + 1
            messages.append({**message, 'event': ids[old]})
        ns['turn'](state, messages, 4)
        actual = p.detached(ns['observe'](state))
        # Reclamation changes history/record projections, never live behavior or outcomes.
        for field in (
            'pending',
            'runnable',
            'ready',
            'queues',
            'terminal',
            'host_cancelled',
            'host_reason',
            'boundary',
        ):
            assert actual[field] == expected[field], (mode, field, actual, expected)
        tail = p.detached(expected['events'][-64:])
        for event in tail:
            if event[0] == 'reject':
                event[1]['event'] = ids[event[1]['event']]
        assert actual['events'] == tail, (mode, actual['events'], tail)
        assert len(actual['events']) <= 64
        assert not state['seen']
        inputs.append(messages)
        views.append(actual)
    return {'mode': 'bounded-' + original['mode'], 'inputs': inputs, 'views': views}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    original_source = p.source()
    bounded_source = original_source.replace("new_driver(config['mode'])", "new_driver(config['mode'], False)")
    cases = []
    for mode in (
        'success',
        'body-error',
        'host-error',
        'host-cancel',
        'host-cancel-cleanup-error',
        'host-wake-cancel',
        'cleanup-error',
        'early',
        'put',
        'get',
        'join',
        'external',
        'suppress',
    ):
        original = p.script(mode)
        cases.append(bounded_case(original, original_source))
    report = {'identity': p.identity(), 'backends': {}, 'status': 'FAIL'}
    p.source = lambda: bounded_source
    try:
        for backend in ('native', 'wasm'):
            report['backends'][backend] = {}
            for case in cases:
                report['backends'][backend][case['mode']] = p.replay(
                    backend, case, args.output / backend / case['mode']
                )
                print(backend, case['mode'], 'PASS', flush=True)
        report['status'] = 'PASS'
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
