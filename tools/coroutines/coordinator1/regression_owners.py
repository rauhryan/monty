"""Required-failure ownership and ordered-input controls from independent review 1."""

import argparse
import json
import types
from pathlib import Path

from coordinator1 import proof as p

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
out = parser.parse_args().output
out.mkdir(exist_ok=False)
base = '\n'.join((p.ROOT / n).read_text() for n in p.FILES[:3])
workload = """
async def child(state,task,mode):
    async with Scope(state,task,None) as scope:
        if mode == 'consumed-cleanup':
            scope.defer('audit', 'child')
        raise ValueError('CHILD-ERROR')
async def parent_body(state,task,mode):
    async with Scope(state,task,None) as scope:
        a=await scope.spawn(child,'a',mode)
        for _ in range(4):
            await scope.checkpoint()
        if mode == 'consumed-cleanup':
            await scope.join(a)
        await scope.operation(['park'])
        return 'parent-value'
"""
report = {}
for mode in ('unconsumed', 'consumed-cleanup'):
    ns = {}
    exec((base + workload).replace(p.DIRECTIVE, ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda b: b.__await__()
    s = ns['new_driver'](mode, False)
    ns['turn'](s, [], 4)
    inputs = [{'mode': mode}]
    views = [p.detached(ns['observe'](s))]
    serial = 0

    def step(msgs):
        ns['turn'](s, msgs, 4)
        for t in s['tasks'].values():
            assert t['owner'] is None or t['owner'] in s['scopes']
            for sid in t['scopes']:
                assert s['scopes'][sid]['task'] == t['id']
        for scope in s['scopes'].values():
            assert scope['task'] in s['tasks']
            assert scope['parent'] is None or scope['parent'] in s['scopes']
            for tid in scope['children']:
                assert s['tasks'][tid]['owner'] == scope['id']
        inputs.append(msgs)
        views.append(p.detached(ns['observe'](s)))

    for _ in range(30):
        if any(x['payload'] == ['park'] for x in views[-1]['pending']):
            break
        msgs = []
        for x in views[-1]['pending']:
            serial += 1
            msgs.append(
                dict(
                    run='C1', event=serial, kind='reply', operation=x['boundary'], outcome='error', value='AUDIT-ERROR'
                )
            )
        step(msgs)
    park = views[-1]['pending'][0]
    assert park['payload'] == ['park']
    assert ('T1' in s['tasks']) == (mode == 'unconsumed')
    for _ in range(100):
        serial += 1
        step([dict(run='C1', event=serial, kind='unsupported')])
    assert not any(e[0] == 'exit' and e[1] == 'T1' for e in s['events'])
    mark = s['event_watermark']
    step([dict(run='WRONG', event=mark + 1000, kind='cancel', reason='bad')])
    assert s['event_watermark'] == mark
    step([dict(run='C1', event='string', kind='cancel', reason='bad')])
    assert s['event_watermark'] == mark
    step(
        [
            dict(run='C1', event=mark + 2, kind='unsupported'),
            dict(run='C1', event=mark + 1, kind='cancel', reason='bad'),
        ]
    )
    assert s['event_watermark'] == mark + 2 and not s['host_cancelled']
    step([dict(run='C1', event=mark, kind='cancel', reason='bad')])
    assert not s['host_cancelled']
    assert s['tasks']['T0']['wait'] == park
    step([dict(run='C1', event=mark + 3, kind='reply', operation=park['boundary'], outcome='value', value=None)])
    for _ in range(20):
        if views[-1]['terminal'] is not None:
            break
        step([])
    result = views[-1]['terminal']
    assert result['kind'] == 'error' and result['children'][0]['cause'] == 'CHILD-ERROR'
    assert result['children'][0]['failures'] == (['AUDIT-ERROR'] if mode == 'consumed-cleanup' else [])
    assert len(s['tasks']) == 1 and not s['scopes']
    case = {'mode': mode, 'inputs': inputs, 'views': views}
    guest = (
        base
        + workload
        + '\ndef adapt(body):\n    return body\n'
        + p.BRIDGE.replace("new_driver(config['mode'])", "new_driver(config['mode'], False)")
    )
    p.source = lambda: guest
    report[mode] = {'terminal': result, 'backends': {}}
    for backend in ('native', 'wasm'):
        report[mode]['backends'][backend] = p.replay(backend, case, out / mode / backend)
        print(mode, backend, 'PASS', flush=True)
(out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
