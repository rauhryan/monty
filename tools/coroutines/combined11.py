"""Combined async-with/owned-child proof on CPython and fresh native/Wasm instances."""

import argparse
import json
import types
from pathlib import Path
from trace import digest, finite_dumps

import scope9
from async10 import DIRECTIVE
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent
RETAINED = []
BRIDGE = """
import json
config = json.loads(exchange(json.dumps({'kind': 'configure', 'contract': 7})))
state = new_driver(config['mode'])
while state['tasks']['T0']['result'] is None:
    message = json.loads(exchange(json.dumps(view(state))))
    if message['kind'] == 'tamper-shadow':
        state['tasks']['T0']['cancel'] = None
    else:
        command(state, message)
json.dumps(view(state))
"""


def guest_source():
    return (
        (ROOT / 'combined11_guest.py').read_text()
        + '\ndef adapt(body):\n    return body\n'
        + (ROOT / 'combined11_driver.py').read_text()
        + BRIDGE
    )


def namespace(syntax):
    ns = {}
    if syntax:
        exec(
            compile(
                (ROOT / 'combined11_guest.py').read_text().replace(DIRECTIVE, '', 1), 'combined11_guest.py', 'exec'
            ),
            ns,
        )
        ns['request'] = types.coroutine(ns['request'])
        ns['adapt'] = lambda body: body
    else:
        exec(compile((ROOT / 'combined11_reference.py').read_text(), 'combined11_reference.py', 'exec'), ns)
        ns['adapt'] = lambda body: body
    exec(compile((ROOT / 'combined11_driver.py').read_text(), 'combined11_driver.py', 'exec'), ns)
    if syntax:
        parent, child = ns['parent_body'], ns['child_body']

        def lazy(function, state, task, mode):
            return (yield from function(state, task, mode).__await__())

        ns['parent_body'] = lambda state, task, mode: lazy(parent, state, task, mode)
        ns['child_body'] = lambda state, task, mode: lazy(child, state, task, mode)
    return ns


class Script:
    """Record explicit host decisions against both independently spelled bodies."""

    def __init__(self, name, mode='normal'):
        self.name = name
        self.ns = [namespace(False), namespace(True)]
        self.states = [n['new_driver'](mode) for n in self.ns]
        self.script = {'name': name, 'mode': mode, 'initial': self.view(), 'turns': []}
        RETAINED.extend(self.states)

    @property
    def state(self):
        return self.states[0]

    def view(self):
        views = [json.loads(finite_dumps(n['view'](s))) for n, s in zip(self.ns, self.states, strict=True)]
        assert views[0] == views[1], (self.name, views)
        return views[0]

    def do(self, kind, task='T0', **fields):
        before = self.view()
        message = {'run': 'R11', 'kind': kind, 'task': task, **fields}
        for n, s in zip(self.ns, self.states, strict=True):
            if kind == 'tamper-shadow':
                s['tasks']['T0']['cancel'] = None
            else:
                n['command'](s, message)
        self.script['turns'].append((message, before, self.view()))

    def step(self, task='T0'):
        self.do('advance', task)

    def settle(self, task='T0', value=None, outcome='value'):
        self.do('settle', task, boundary=self.state['tasks'][task]['wait']['boundary'], value=value, outcome=outcome)

    def cancel(self, reason='alarm'):
        self.do('cancel', scope='S0', reason=reason)

    def setup(self, join=True):
        self.step()
        self.step()
        if join:
            self.step()
        for task in ('T1', 'T2'):
            self.step(task)
            self.settle(task, task + '-H')
            self.step(task)

    def finish(self, task, value='OK', error=False, cleanup_error=False):
        self.settle(task, value, 'error' if error else 'value')
        self.step(task)  # __aexit__ drain request, no resource operation yet
        self.step(task)
        self.settle(task, 'CFAIL' if cleanup_error else None, 'error' if cleanup_error else 'value')
        self.step(task)
        self.settle(task)
        self.step(task)

    def drain(self):
        # This is a host test script selecting every individual step, not a guest scheduler.
        for _ in range(60):
            if self.state['tasks']['T0']['result'] is not None:
                return
            children = [t for t in self.state['tasks'].values() if t['id'] != 'T0' and t['status'] != 'done']
            task = children[-1] if children else self.state['tasks']['T0']
            if task['wait'] is not None:
                self.settle(task['id'])
            else:
                assert task['status'] == 'runnable', task
                self.step(task['id'])
        raise AssertionError('bounded script did not finish')


def scenarios():
    scripts = []
    for order in [('T1', 'T2'), ('T2', 'T1')]:
        s = Script('completion-' + '-'.join(order))
        s.setup()
        for task in order:
            s.finish(task, task + '-value')
        s.step()
        s.step()
        assert s.state['tasks']['T0']['phase'] == 'drain'
        s.drain()
        assert s.view()['terminal']['kind'] == 'value'
        scripts.append(s.script)
    for mode in ('early', 'early-live', 'error', 'suppress', 'break', 'catch'):
        s = Script(mode, mode)
        if mode == 'early':
            s.step()
            s.step()
            s.step()
        else:
            s.setup()
            if mode == 'catch':
                s.cancel()
                s.step()
            else:
                s.settle(value='parent')
                s.step()
        assert s.state['tasks']['T0']['phase'] == 'drain'
        old_resume = list(s.state['tasks']['T0']['resume'])
        s.do('spawn', scope='S0')
        s.cancel('late')
        s.cancel('repeat')
        assert s.state['tasks']['T0']['resume'] == old_resume
        s.drain()
        end = s.view()['terminal']
        assert end['kind'] == 'cancelled'
        assert end['cause'] == ('body-original' if mode == 'error' else 'alarm' if mode == 'catch' else 'late')
        if mode == 'catch':
            assert end['value'] == 'caught-parent-success'
        scripts.append(s.script)
    s = Script('nested-normal', 'nested')
    s.step()
    s.settle(value='inner')
    s.step()
    s.step()
    s.settle()
    s.step()  # first spawn
    s.step()  # second spawn
    s.step()  # join
    for task in ('T2', 'T1'):
        s.step(task)
        s.settle(task, task + '-H')
        s.step(task)
        s.finish(task)
    s.step()
    s.step()
    s.drain()
    assert s.view()['terminal']['kind'] == 'value'
    scripts.append(s.script)
    for already_done in (False, True):
        for preempt in (False, True):
            s = Script(f'join-{already_done}-preempt-{preempt}', 'ignore-error')
            s.setup(join=not already_done)
            s.finish('T1', 'BODY', error=True)
            if already_done:
                s.step()
            assert not s.state['tasks']['T1']['consumed']
            if preempt:
                s.cancel()
            s.step()
            assert s.state['tasks']['T1']['consumed'] == (not preempt)
            if not preempt:
                s.finish('T2')
                s.step()
            s.drain()
            assert len(s.view()['terminal']['children']) == int(preempt)
            scripts.append(s.script)
    for mode in ('normal', 'ignore-error'):
        s = Script('cleanup-only-' + mode, mode)
        s.setup()
        s.finish('T1', cleanup_error=True)
        s.step()
        if mode == 'ignore-error':
            s.finish('T2')
            s.step()
        s.drain()
        end = s.view()['terminal']
        assert end['kind'] == 'error'
        assert end['cause'] == ('child:failed' if mode == 'normal' else None)
        assert end['children'][0]['failures'] == ['CFAIL']
        scripts.append(s.script)
    s = Script('orphan-acquire')
    s.step()
    s.step()
    s.step()
    s.step('T1')
    s.settle('T1', 'ORPHAN')
    s.cancel()
    s.step()
    s.do('settle', 'T1', boundary='B0', value='stale', outcome='value')
    s.drain()
    assert [e[1]['payload'] for e in s.state['events'] if e[0] == 'pause'] == [
        ['acquire', 'a'],
        ['parent-arbitrary', 'P'],
    ]
    scripts.append(s.script)
    scripts.extend(extra_scenarios())
    return scripts


def extra_scenarios():
    scripts = []
    for mode in ('early', 'early-live', 'error', 'suppress', 'break'):
        s = Script('uncancelled-' + mode, mode)
        if mode == 'early':
            s.step()
            s.step()
            s.step()
        else:
            s.setup()
            s.settle(value='parent')
            s.step()
        s.drain()
        end = s.view()['terminal']
        assert end == {
            'kind': 'error' if mode == 'error' else 'value',
            'value': {
                'early': 'early',
                'early-live': 'early-live',
                'error': None,
                'suppress': 'after-scope',
                'break': 'after-scope',
            }[mode],
            'cause': 'body-original' if mode == 'error' else None,
            'cancel': None,
            'failures': [],
            'children': [],
        }
        scripts.append(s.script)
    s = Script('multiple-failures-original-cause', 'error')
    s.setup()
    s.settle(value='parent')
    s.step()
    assert s.state['scopes']['S0']['cause'] == 'body-original'
    for task in ('T2', 'T1'):
        s.step(task)
        s.step(task)
        cursor = s.state['scopes'][s.state['tasks'][task]['stack'][-1]]['cursor']
        pending = dict(s.state['tasks'][task]['wait'])
        s.cancel('late-cancel')
        s.do('settle', task, boundary='B404', value='stale', outcome='value')
        assert s.state['tasks'][task]['wait'] == pending
        s.settle(task, task + '-C1', 'error')
        resume = list(s.state['tasks'][task]['resume'])
        s.cancel('repeat-after-settle')
        assert s.state['tasks'][task]['resume'] == resume
        assert s.state['scopes'][s.state['tasks'][task]['stack'][-1]]['cursor'] == cursor
        s.step(task)
        s.settle(task, task + '-C2', 'error')
        s.step(task)
    s.step()
    s.settle(value='PFAIL', outcome='error')
    s.cancel('repeat-parent-cleanup')
    s.step()
    end = s.view()['terminal']
    assert end == {
        'kind': 'cancelled',
        'value': None,
        'cause': 'body-original',
        'cancel': 'late-cancel',
        'failures': ['PFAIL'],
        'children': [
            {
                'kind': 'cancelled',
                'value': None,
                'cause': 'owner-exit:S0',
                'cancel': 'owner-exit:S0',
                'failures': [task + '-C1', task + '-C2'],
                'children': [],
            }
            for task in ('T1', 'T2')
        ],
    }
    scripts.append(s.script)
    s = Script('authority_veto_guest_metadata_mutation', 'catch')
    s.setup()
    s.cancel()
    s.step()
    for task in ('T2', 'T1'):
        s.step(task)
        s.step(task)
        s.settle(task)
        s.step(task)
        s.settle(task)
        s.step(task)
    s.step()
    s.settle()
    s.do('tamper-shadow')
    s.step()
    assert s.view()['terminal'] == {
        'kind': 'value',
        'value': 'caught-parent-success',
        'cause': None,
        'cancel': None,
        'failures': [],
        'children': [],
    }
    scripts.append(s.script)
    s = Script('unconsumed-body-error', 'early-live')
    s.setup()
    s.finish('T1', 'UNCONSUMED', error=True)
    assert s.state['tasks']['T2']['status'] == 'wait'
    assert s.state['tasks']['T0']['status'] == 'wait'
    assert s.state['tasks']['T0']['cancel'] is None
    s.settle(value='parent')
    s.step()
    s.drain()
    assert s.view()['terminal']['kind'] == 'error'
    assert s.view()['terminal']['children'] == [
        {'kind': 'error', 'value': None, 'cause': 'UNCONSUMED', 'cancel': None, 'failures': [], 'children': []}
    ]
    scripts.append(s.script)
    s = Script('nested-owned-child', 'nested')
    s.step()  # inner body operation
    s.do('spawn', scope='S1')
    s.step('T1')
    s.settle('T1', 'INNER-H')
    s.step('T1')
    s.settle(value='inner-done')
    s.step()  # inner __aexit__ must drain its own live child
    assert s.state['tasks']['T0']['draining'] == 'S1'
    s.step('T1')
    s.step('T1')
    s.settle('T1')
    s.step('T1')
    s.settle('T1')
    s.step('T1')
    s.step()
    assert s.state['tasks']['T0']['wait']['payload'] == ['inner-arbitrary', 'I']
    s.settle()
    s.step()
    s.step()
    s.step()
    assert s.state['tasks']['T1']['owner'] == 'S1'
    assert s.state['tasks']['T2']['owner'] == 'S0'
    assert s.state['scopes']['S1']['parent'] == 'S0'
    s.do('settle', 'T2', run='obsolete-run', boundary='B404', value='late', outcome='value')
    s.do('spawn', scope='S404')
    s.cancel()
    s.step()
    s.drain()
    assert s.view()['terminal']['kind'] == 'cancelled'
    scripts.append(s.script)
    return scripts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    report = {
        'status': 'FAIL',
        'runtime_sha256': scope9.runtime_identity(),
        'full_source_sha256': source_identity(),
        'cases': {},
        'inputs': {p.name: digest(p) for p in ROOT.glob('combined11*.py')},
    }
    with args.report.open('x') as output:
        try:
            baseline = json.loads((ROOT / 'async10_artifacts.json').read_text())
            assert baseline['runtime_sha256'] == report['runtime_sha256']
            report['association'] = baseline
            scripts = scenarios()
            scope9.guest_source = guest_source
            for backend in ('native', 'wasm'):
                assert {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)} == baseline[backend][
                    'artifacts'
                ]
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in scripts:
                    path = args.report.parent / (args.report.stem + '-cuts') / backend / script['name']
                    report['cases'].setdefault(script['name'], {})[backend] = run(command, script, path)
                    print(backend, script['name'], 'PASS', flush=True)
            report['status'] = 'PASS'
        except (AssertionError, ValueError, OSError) as error:
            report['error'] = str(error)
            print('FAIL', error, flush=True)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


def run(command, script, directory):
    # Reuse the physical recovery adapter, with R11-specific authority assertions below.
    result = scope9.run(command, script, directory, 30)
    if script['name'] == 'orphan-acquire':
        assert len(result['authority']['reconciled']) == 1
        assert result['authority']['reconciled'][0]['value'] == 'ORPHAN'
    return result


if __name__ == '__main__':
    raise SystemExit(main())
