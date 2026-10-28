"""Independent cancellation ownership literals, including reclaimed children."""

import argparse
import json
import types
from pathlib import Path

from coordinator1 import proof as p

WORKLOAD = """
async def child(state, task, mode):
    async with Scope(state, task, None) as scope:
        if mode == 'repeat':
            await scope.operation(['hold'])
        return 'child-value'

async def invalid(scope, handle):
    try:
        await scope.cancel(handle, 'INVALID')
    except ValueError as error:
        assert str(error) == 'invalid-cancel'
        return
    assert False, 'invalid cancellation accepted'

async def parent_body(state, task, mode):
    async with Scope(state, task, None) as scope:
        a = await scope.spawn(child, 'a', mode)
        if mode == 'foreign':
            await scope.join(a)
            return a
        if mode == 'repeat':
            await scope.checkpoint()
            await scope.cancel(a, 'first')
        result = await scope.join(a)
        assert result['kind'] == ('cancelled' if mode == 'repeat' else 'value')
        assert result['cause'] == ('task-cancel:first' if mode == 'repeat' else None)
        for _ in range(40):
            await scope.checkpoint()
        await scope.cancel(a, 'delayed-40')
        for _ in range(40):
            await scope.checkpoint()
        assert not any(e[0] == 'exit' and e[1] == 'T1' for e in state['events']) or state['retain_history']
        assert 'T1' not in state['tasks'] or state['retain_history']
        await scope.cancel(a, 'delayed')
        # Allocated IDs, never-issued IDs, and lookalike identities confer no authority.
        await invalid(scope, 'T1')
        await invalid(scope, 'T999999')
        await invalid(scope, None)
        await invalid(scope, ChildHandle('T1', []))
        await invalid(scope, ChildHandle('T999999', scope.scope['identity']))
        foreign = new_driver('foreign', False)
        for _ in range(20):
            if foreign['tasks']['T0']['status'] == 'done':
                break
            turn(foreign, [], 4)
        foreign_child = foreign['tasks']['T0']['result']['value']
        assert foreign_child._id == a._id
        assert foreign_child._done
        await invalid(scope, foreign_child)
        try:
            await scope.join(a)
        except ValueError as error:
            assert str(error) == 'invalid-join:S0'
        else:
            assert False, 'repeat join accepted'
        async with Scope(state, task, None) as other:
            await invalid(other, a)
            b = await other.spawn(child, 'b', 'repeat')
            for _ in range(4):
                await other.checkpoint()
            await invalid(scope, b)
            await invalid(other, b._id)
            await invalid(other, ChildHandle(b._id, other.scope['identity']))
            await other.cancel(b, 'live')
            result = await other.join(b)
            assert result['kind'] == 'cancelled'
            assert result['cause'] == 'task-cancel:live'
            for _ in range(40):
                await other.checkpoint()
            await invalid(scope, b)
            await other.cancel(b, 'again')
        await invalid(scope, b)
        return 'parent-value'
"""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    base = '\n'.join((p.ROOT / n).read_text() for n in p.FILES[:3])
    report = {'identity': p.identity(), 'cases': {}}
    for history in (True, False):
        for mode in ('done', 'repeat'):
            ns = {}
            exec((base + WORKLOAD).replace(p.DIRECTIVE, ''), ns)
            ns['request'] = types.coroutine(ns['request'])
            ns['adapt'] = lambda body: body.__await__()
            state = ns['new_driver'](mode, history)
            ns['turn'](state, [], 4)
            inputs = [{'mode': mode}]
            views = [p.detached(ns['observe'](state))]
            for _ in range(150):
                if views[-1]['terminal'] is not None:
                    break
                ns['turn'](state, [], 4)
                inputs.append([])
                views.append(p.detached(ns['observe'](state)))
            terminal = views[-1]['terminal']
            assert terminal == {
                'kind': 'value',
                'value': 'parent-value',
                'cause': None,
                'cancel': None,
                'failures': [],
                'children': [],
            }, terminal
            if not history:
                assert len(state['tasks']) == 1 and not state['scopes']
            case = {'mode': mode, 'inputs': inputs, 'views': views}
            guest = (
                base
                + WORKLOAD
                + '\ndef adapt(body):\n    return body\n'
                + p.BRIDGE.replace("new_driver(config['mode'])", f"new_driver(config['mode'], {history})")
            )
            p.source = lambda: guest
            key = f'{history}-{mode}'
            report['cases'][key] = {'terminal': terminal, 'backends': {}}
            for backend in ('native', 'wasm'):
                report['cases'][key]['backends'][backend] = p.replay(backend, case, args.output / key / backend)
                print(key, backend, 'PASS', flush=True)
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
