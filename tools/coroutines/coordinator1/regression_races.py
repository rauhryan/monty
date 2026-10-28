import argparse
import ast
import json
import types
from pathlib import Path

from coordinator1 import proof as p
from coordinator1.retention_checks import bounded_case

parser = argparse.ArgumentParser(description='Maintained independent literal race controls')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--bounded', action='store_true')
args = parser.parse_args()
OUT = args.output
OUT.mkdir(exist_ok=False)
base = '\n'.join((p.ROOT / n).read_text() for n in p.FILES[:3])
tree = ast.parse((p.ROOT / 'regression_probes.py').read_text())
custom = next(
    ast.literal_eval(n.value)
    for n in tree.body
    if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'CUSTOM' for t in n.targets)
)
report = {'identity': p.identity(), 'cases': {}, 'backends': {}}
cases = []
for mode in ['external-queued', 'external-delivered', 'reserved-get', 'reserved-put', 'protected-guest']:
    workload = custom
    if mode.startswith('reserved'):
        workload = """
async def waiter(state,task,mode):
    async with Scope(state,task,None) as scope:
        if mode=='reserved-get':
            return await scope.get('q')
        return await scope.put('q','NEW')
async def parent_body(state,task,mode):
    state['queues']={'q':{'items':[],'get':[],'put':[]}}
    async with Scope(state,task,None) as scope:
        if mode=='reserved-put':
            state['queues']['q']['items'].append('OLD')
        a=await scope.spawn(waiter,'a',mode)
        for _ in range(0):
            await scope.checkpoint()
        if mode=='reserved-get':
            await scope.put('q','ONLY')
        else:
            await scope.get('q')
        return await scope.join(a)
"""
    elif mode == 'protected-guest':
        workload = (
            custom[: custom.index('async def parent_body')]
            + """
async def parent_body(state,task,mode):
    async with Scope(state,task,None) as scope:
        a=await scope.spawn(child,'a','nested')
        await scope.operation(['gate'])
        await scope.cancel(a,'masked')
        return await scope.join(a)
"""
        )
    ns = {}
    exec((base + '\n' + workload).replace(p.DIRECTIVE, ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda b: b.__await__()
    s = ns['new_driver'](mode)
    ns['turn'](s, [], 4)
    inputs = [{'mode': mode}]
    views = [p.detached(ns['observe'](s))]
    serial = 0
    fired = False
    for i in range(80):
        if s['tasks']['T0']['result'] is not None:
            break
        msgs = []
        pending = list(views[-1]['pending'])
        if mode == 'external-delivered':
            pending.reverse()
        for op in pending:
            if (
                mode == 'protected-guest'
                and op['payload'][0] == 'inner'
                and not any(e[0] == 'advance' and e[1] == 'T0' for e in s['events'][-2:])
            ):
                # Hold inner cleanup until parent has processed its cancellation request.
                if s['tasks']['T0']['status'] != 'join':
                    continue
            serial += 1
            msgs.append(
                {
                    'run': 'C1',
                    'event': serial,
                    'kind': 'reply',
                    'operation': op['boundary'],
                    'outcome': 'value',
                    'value': 'ok',
                }
            )
        if mode.startswith('reserved') and not fired:
            t = s['tasks'].get('T1')
            if t and t['io'] is not None and t['status'] == 'runnable':
                serial += 1
                msgs.append({'run': 'C1', 'event': serial, 'kind': 'cancel', 'reason': ''})
                fired = True
        ns['turn'](s, msgs, 4)
        inputs.append(msgs)
        views.append(p.detached(ns['observe'](s)))
    end = views[-1]
    assert end['terminal'] is not None
    if mode.startswith('reserved'):
        # Explicitly report if four-resume cut did not expose the desired reservation.
        assert fired, 'reserved wake cut not reached'
        assert end['queues']['q']['items'] == (['ONLY'] if mode == 'reserved-get' else [])
        assert not any(e[0] in ('put', 'get') and e[1] == 'T1' for e in end['events'])
    elif mode == 'protected-guest':
        assert ['body-signal', 'task-cancel:masked'] not in end['events']
        assert end['tasks'][1][6]['kind'] == 'value'
        assert any(e[0] == 'pause' and e[1]['payload'] == ['after-inner'] for e in end['events'])
    else:
        delivered = ['delivered', 'a'] in end['events']
        assert delivered == (mode == 'external-delivered'), (mode, delivered)
    case = {'mode': mode, 'inputs': inputs, 'views': views}
    cases.append((case, base + '\n' + workload + '\ndef adapt(body):\n    return body\n' + p.BRIDGE))
    report['cases'][mode] = case
try:
    for backend in ['native', 'wasm']:
        report['backends'][backend] = {}
        for case, guest in cases:
            if args.bounded:
                case = bounded_case(case, guest)
                guest = guest.replace("new_driver(config['mode'])", "new_driver(config['mode'], False)")
            p.source = lambda guest=guest: guest
            report['backends'][backend][case['mode']] = p.replay(backend, case, OUT / backend / case['mode'])
            print(backend, case['mode'], 'PASS', flush=True)
finally:
    (OUT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
