import argparse
import json
import types
from pathlib import Path

from coordinator1 import proof as p
from coordinator1.retention_checks import bounded_case

parser = argparse.ArgumentParser(description='Maintained independent literal policy controls')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--bounded', action='store_true')
args = parser.parse_args()
OUT = args.output
OUT.mkdir(exist_ok=False)
BASE = '\n'.join((p.ROOT / n).read_text() for n in p.FILES[:3])
CUSTOM = """
async def child(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', task['label'])
        if mode == 'failure':
            raise ValueError('UNCONSUMED')
        if mode == 'nested':
            async with Scope(state, task, None) as inner:
                inner.defer('inner', task['label'])
                await inner.checkpoint()
            state['events'].append(['back-in-body'])
            try:
                await scope.operation(['after-inner'])
            except ValueError as error:
                state['events'].append(['body-signal', str(error)])
            return 'continued'
        try:
            if mode == 'put':
                await scope.put('q', task['label'])
            elif mode == 'get':
                value = await scope.get('q')
                state['events'].append(['received', task['label'], value])
            else:
                await scope.operation(['leaf', task['label']])
            state['events'].append(['delivered', task['label']])
        except ValueError as error:
            state['events'].append(['signal', task['label'], str(error)])
            if mode != 'suppress':
                raise
        return 'child-value'

async def parent_body(state, task, mode):
    state['queues'] = {'q': {'items': [], 'get': [], 'put': []}}
    async with Scope(state, task, None) as scope:
        scope.defer('close', 'parent')
        if mode in ('queue-get', 'queue-put'):
            kind = 'get' if mode == 'queue-get' else 'put'
            if kind == 'put':
                state['queues']['q']['items'].append('initial')
            a = await scope.spawn(child, 'a', kind)
            b = await scope.spawn(child, 'b', kind)
            c = await scope.spawn(child, 'c', kind)
            for _ in range(5):
                await scope.checkpoint()
            await scope.cancel(a, 'first')
            await scope.cancel(b, 'second')
            if kind == 'get':
                await scope.put('q', 'ONLY')
            else:
                assert await scope.get('q') == 'initial'
            result = await scope.join(c)
            if kind == 'put':
                assert await scope.get('q') == 'c'
            return result
        if mode in ('join-before', 'join-after', 'unjoined'):
            a = await scope.spawn(child, 'a', 'failure')
            for _ in range(3):
                await scope.checkpoint()
            await scope.operation(['gate'])
            for _ in range(3 if mode == 'join-before' else 0):
                await scope.checkpoint()
            if mode != 'unjoined':
                result = await scope.join(a)
                state['events'].append(['joined-review', result])
            await scope.operation(['after-join'])
            return 'parent-value'
        if mode == 'nested':
            a = await scope.spawn(child, 'a', 'nested')
            return await scope.join(a)
        a = await scope.spawn(child, 'a', 'suppress')
        await scope.operation(['gate'])
        await scope.cancel(a, 'guest')
        return await scope.join(a)
"""


def make_case(name, mode, custom=True, trigger=None, order='forward'):
    source = BASE + '\n' + (CUSTOM if custom else (p.ROOT / 'workload.py').read_text())
    ns = {}
    exec(source.replace(p.DIRECTIVE, ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda body: body.__await__()
    s = ns['new_driver'](mode)
    ns['turn'](s, [], 4)
    views = [p.detached(ns['observe'](s))]
    inputs = [{'mode': mode}]
    serial = 0
    fired = False
    for step in range(100):
        if s['tasks']['T0']['result'] is not None:
            break
        msgs = []
        pending = list(views[-1]['pending'])
        if order == 'reverse':
            pending.reverse()
        # Supply replies even when guest work remains runnable.
        if pending:
            chosen = pending if order != 'single' else pending[:1]
            for op in chosen:
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
        if trigger and not fired and trigger(s, views[-1]):
            serial += 1
            msgs.append({'run': 'C1', 'event': serial, 'kind': 'cancel', 'reason': ''})
            fired = True
        ns['turn'](s, msgs, 4)
        inputs.append(msgs)
        views.append(p.detached(ns['observe'](s)))
    assert s['tasks']['T0']['result'] is not None
    assert not trigger or fired
    case = {'mode': name, 'inputs': inputs, 'views': views}
    guest = source + '\ndef adapt(body):\n    return body\n' + p.BRIDGE
    return case, guest


report = {'identity': p.identity(), 'cases': {}, 'backends': {}}
CASES = []
for mode in ['queue-get', 'queue-put', 'unjoined', 'nested', 'join-before', 'join-after', 'guest-host']:
    trigger = None
    if mode == 'nested':

        def trigger(s, v):
            return any(x['payload'][0] == 'inner' for x in v['pending'])

    if mode == 'join-before':

        def trigger(s, v):
            return s['tasks']['T0']['pending_join'] is not None

    if mode == 'join-after':

        def trigger(s, v):
            return any(x['payload'][0] == 'after-join' for x in v['pending'])

    if mode == 'guest-host':

        def trigger(s, v):
            return any(t['guest_signal'] is not None for t in s['tasks'].values())

    case, guest = make_case(mode, mode, trigger=trigger)
    end = case['views'][-1]
    ev = end['events']
    if mode == 'queue-get':
        assert [x for x in ev if x[0] == 'received'] == [['received', 'c', 'ONLY']]
        assert end['queues']['q'] == {'items': [], 'get': [], 'put': []}
    if mode == 'queue-put':
        assert [x for x in ev if x[0] == 'put'] == [['put', 'T3', 'c']]
        assert end['queues']['q'] == {'items': [], 'get': [], 'put': []}
    if mode == 'unjoined':
        assert end['terminal']['kind'] == 'error'
        assert end['terminal']['children'][0]['cause'] == 'UNCONSUMED'
    if mode == 'nested':
        assert ['body-signal', ''] in ev
        assert not any(x[0] == 'pause' and x[1]['payload'][0] == 'after-inner' for x in ev)
    if mode == 'join-before':
        assert not end['tasks'][1][5]
        assert end['terminal']['children'][0]['cause'] == 'UNCONSUMED'
    if mode == 'join-after':
        assert end['tasks'][1][5]
        assert not end['terminal']['children']
    if mode == 'guest-host':
        assert end['host_cancelled'] and end['terminal']['kind'] == 'cancelled'
    CASES.append((case, guest))
    report['cases'][mode] = case
for order in ['forward', 'reverse', 'single']:
    case, guest = make_case('eager-' + order, 'success', False, order=order)
    ev = case['views'][-1]['events']
    assert case['views'][-1]['terminal']['kind'] == 'value'
    assert [x[2] for x in ev if x[0] == 'put'] == ['A', 'B', 'C', None, None]
    assert sorted(x[2] for x in ev if x[0] == 'get' and x[2] is not None) == ['A', 'B', 'C']
    CASES.append((case, guest))
    report['cases'][case['mode']] = case
try:
    for backend in ['native', 'wasm']:
        report['backends'][backend] = {}
        for case, guest in CASES:
            if args.bounded:
                case = bounded_case(case, guest)
                guest = guest.replace("new_driver(config['mode'])", "new_driver(config['mode'], False)")
            p.source = lambda guest=guest: guest
            report['backends'][backend][case['mode']] = p.replay(backend, case, OUT / backend / case['mode'])
            print(backend, case['mode'], 'PASS', flush=True)
finally:
    (OUT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
