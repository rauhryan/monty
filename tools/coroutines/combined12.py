"""Literal R11-1 policy regressions, replayed across fresh native/actual-Wasm cuts."""

import argparse
import json
from pathlib import Path
from trace import digest, finite_dumps

import combined11 as c
import scope9
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent
REASON = 'LATE-INNER'
CANCELLED = {'kind': 'cancelled', 'value': None, 'cause': REASON, 'cancel': REASON, 'failures': [], 'children': []}


def pause(boundary, scope, parent, phase, payload):
    """Spell fixed effect metadata without observing the driver."""
    return [
        'pause',
        {
            'run': 'R11',
            'task': 'T0',
            'scope': scope,
            'parent_scope': parent,
            'boundary': boundary,
            'phase': phase,
            'payload': payload,
        },
    ]


def literal_events(stage):
    """Independently specified complete trace for the two supplied-body failures."""
    events = [
        ['task', 'T0', None],
        ['advance', 'T0'],
        ['scope', 'S0', 'T0', 50],
        ['scope', 'S1', 'T0', 25],
        pause('B0', 'S1', 'S0', 'body', ['nested-work']),
        ['accept', 'T0', 'B0', 'value', 'inner'],
        ['advance', 'T0'],
        ['drain', 'S1', None],
        ['drained', 'S1'],
    ]
    cleanup = [
        ['advance', 'T0'],
        pause('B1', 'S1', 'S0', 'resource-cleanup', ['inner-arbitrary', 'I']),
        ['accept', 'T0', 'B1', 'value', 'ACCEPTED-INNER-CLEANUP'],
    ]
    cancel = [['cancel', 'T0', REASON]]
    events += cancel + cleanup if stage == 'drain' else cleanup + cancel
    return events + [
        ['advance', 'T0'],
        ['scope-exit', 'S1', None, []],
        ['advance', 'T0'],
        ['drain', 'S0', REASON],
        ['drained', 'S0'],
        ['advance', 'T0'],
        pause('B2', 'S0', None, 'resource-cleanup', ['parent-arbitrary', 'P']),
        ['accept', 'T0', 'B2', 'value', None],
        ['advance', 'T0'],
        ['scope-exit', 'S0', REASON, []],
        ['exit', 'T0', CANCELLED],
    ]


def nested(stage, children=False):
    """Use the original supplied body, including its two post-inner spawn awaits."""
    s = c.Script('repair12-nested-' + stage + ('-children' if children else ''), 'nested')
    s.step()
    if children:
        # Existing fixed child factory: one child belongs to each lexical scope.
        s.do('spawn', scope='S0')
        s.do('spawn', scope='S1')
        for child in ('T1', 'T2'):
            s.step(child)
            s.settle(child, child + '-H')
            s.step(child)
    s.settle(value='inner')
    s.step()
    if children:
        s.step('T2')
        s.step('T2')
        s.settle('T2')
        s.step('T2')
        s.settle('T2')
        s.step('T2')
    if stage == 'queued-cleanup':
        s.step()
        s.settle(value='ACCEPTED-INNER-CLEANUP')
    saved = list(s.state['tasks']['T0']['resume'])
    cursor = s.state['scopes']['S1']['cursor']
    s.do('cancel', scope='S1', reason=REASON)
    assert s.state['tasks']['T0']['cancel_pending']
    assert s.state['tasks']['T0']['resume'] == saved
    assert s.state['scopes']['S1']['cursor'] == cursor
    if stage == 'drain':
        s.step()
        s.settle(value='ACCEPTED-INNER-CLEANUP')
    s.step()  # accepted reply, inner exit, first body request becomes throw boundary
    assert s.state['tasks']['T0']['resume'] == ['error', REASON]
    assert not s.state['tasks']['T0']['cancel_pending']
    assert s.state['scopes']['S1']['phase'] == 'exited'
    assert s.state['scopes']['S0']['phase'] == 'closing'
    s.step()  # throw into spawn await, unwind outer context into its drain
    assert s.state['tasks']['T0']['phase'] == 'drain'
    if children:
        assert s.state['tasks']['T0']['status'] == 'drain'
        s.step('T1')
        s.step('T1')
        s.settle('T1')
        s.step('T1')
        s.settle('T1')
        s.step('T1')
    s.drain()
    assert s.view()['terminal'] == CANCELLED
    if children:
        payloads = [e[1]['payload'] for e in s.view()['events'] if e[0] == 'pause']
        assert payloads == [
            ['nested-work'],
            ['acquire', 'late'],
            ['use', 'T1-H'],
            ['acquire', 'late'],
            ['use', 'T2-H'],
            ['another-name', 'T2-H'],
            ['arbitrary-child', 'T2-H'],
            ['inner-arbitrary', 'I'],
            ['another-name', 'T1-H'],
            ['arbitrary-child', 'T1-H'],
            ['parent-arbitrary', 'P'],
        ]
        assert list(s.state['tasks']) == ['T0', 'T1', 'T2']
    else:
        expected = literal_events(stage)
        assert s.view()['events'] == expected
        for _, _, observed in s.script['turns']:
            assert observed['events'] == expected[: len(observed['events'])]
        s.script['literal_events'] = expected
    return s.script


# These are bounded test bodies, assembled only by this runner. The real library,
# driver and fixed child factory are unchanged. Separate generator spelling below.
ASYNC = """
async def parent_body(state, task, mode):
    async with Timebox(state, task, 50) as tg:
        tg.defer('parent-arbitrary', 'P')
        if mode == 'invalid-join':
            try:
                await tg.join(None)
            except ValueError as error:
                state['events'].append(['caught-invalid', str(error)])
            return 'invalid-caught'
        if mode == 'deep':
            async with Timebox(state, task, 30) as middle:
                middle.defer('middle', 'M')
                async with Timebox(state, task, 20) as inner:
                    inner.defer('inner', 'I')
                    await inner.operation(['gate'])
                # No body request: a second protected exit must also finish.
            await tg.operation(['after-deep'])
        try:
            await tg.operation(['gate'])
        except ValueError as error:
            state['events'].append(['caught-cancel', str(error)])
        try:
            await tg.spawn('forbidden', 'normal')
        except ValueError as error:
            state['events'].append(['caught-spawn', str(error)])
        try:
            await tg.join(None)
        except ValueError as error:
            state['events'].append(['caught-join', str(error)])
        return 'caught-success'
"""
REFERENCE = """
def parent_body(state, task, mode):
    scope = open_scope(state, task, 50)
    scope['deferred'].append(['parent-arbitrary', 'P'])
    try:
        if mode == 'invalid-join':
            try:
                yield {'kind': 'join', 'scope': scope['id'], 'child': None}
            except ValueError as error:
                state['events'].append(['caught-invalid', str(error)])
            result = 'invalid-caught'
        else:
            if mode == 'deep':
                middle = open_scope(state, task, 30)
                middle['deferred'].append(['middle', 'M'])
                inner = open_scope(state, task, 20)
                inner['deferred'].append(['inner', 'I'])
                yield {'kind': 'operation', 'scope': inner['id'], 'payload': ['gate']}
                yield from resource_exit(state, task, inner, None)
                yield from resource_exit(state, task, middle, None)
                yield {'kind': 'operation', 'scope': scope['id'], 'payload': ['after-deep']}
            try:
                yield {'kind': 'operation', 'scope': scope['id'], 'payload': ['gate']}
            except ValueError as error:
                state['events'].append(['caught-cancel', str(error)])
            try:
                yield {'kind': 'spawn', 'scope': scope['id'], 'label': 'forbidden', 'mode': 'normal'}
            except ValueError as error:
                state['events'].append(['caught-spawn', str(error)])
            try:
                yield {'kind': 'join', 'scope': scope['id'], 'child': None}
            except ValueError as error:
                state['events'].append(['caught-join', str(error)])
            result = 'caught-success'
    except Exception as error:
        yield from resource_exit(state, task, scope, str(error))
        raise
    else:
        yield from resource_exit(state, task, scope, None)
        return result
"""


def original_error():
    """A pending body error outranks cancellation; accepted cleanup errors accumulate."""
    s = c.Script('repair12-original-error', 'nested')
    s.step()
    s.settle(value='INNER-ORIGINAL', outcome='error')
    s.step()
    s.step()
    s.settle(value='INNER-CLEANUP', outcome='error')
    s.do('cancel', scope='S1', reason=REASON)
    s.cancel('repeat-inner')
    assert s.state['tasks']['T0']['resume'] == ['error', 'INNER-CLEANUP']
    s.step()
    assert s.state['scopes']['S0']['cause'] == 'INNER-ORIGINAL'
    s.step()
    s.settle(value='OUTER-CLEANUP', outcome='error')
    s.cancel('repeat-outer')
    s.step()
    assert s.view()['terminal'] == {
        **CANCELLED,
        'cause': 'INNER-ORIGINAL',
        'failures': ['INNER-CLEANUP', 'OUTER-CLEANUP'],
    }
    assert [e for e in s.view()['events'] if e[0] == 'scope-exit'] == [
        ['scope-exit', 'S1', 'INNER-ORIGINAL', ['INNER-CLEANUP']],
        ['scope-exit', 'S0', 'INNER-ORIGINAL', ['OUTER-CLEANUP']],
    ]
    return s.script


class FixtureScript(c.Script):
    """Retain both explicit fixture continuations, without changing library admission."""

    def __init__(self, mode):
        self.name = 'repair12-' + mode
        self.ns = [c.namespace(False), c.namespace(True)]
        exec(compile(REFERENCE, 'repair12_reference', 'exec'), self.ns[0])
        exec(compile(ASYNC, 'repair12_async', 'exec'), self.ns[1])
        parent = self.ns[1]['parent_body']

        def lazy(state, task, mode):
            return (yield from parent(state, task, mode).__await__())

        self.ns[1]['parent_body'] = lazy
        self.states = [n['new_driver'](mode) for n in self.ns]
        self.script = {'name': self.name, 'mode': mode, 'initial': self.view(), 'turns': [], 'fixture': True}
        c.RETAINED.extend(self.states)


def fixtures():
    s = FixtureScript('admission')
    s.step()
    s.cancel(REASON)
    s.step()  # catches first cancellation, asks to spawn on closed scope
    assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:S0']
    s.step()  # catches spawn rejection, requests join on closed scope
    assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:S0']
    assert s.state['tasks']['T0']['pending_join'] is None
    s.cancel('repeat')
    s.step()  # catches join rejection and returns via drain/cleanup
    s.drain()
    assert s.view()['terminal'] == {**CANCELLED, 'value': 'caught-success'}
    assert [e for e in s.view()['events'] if e[0].startswith('caught')] == [
        ['caught-cancel', REASON],
        ['caught-spawn', 'scope-closed:S0'],
        ['caught-join', 'scope-closed:S0'],
    ]
    assert list(s.state['tasks']) == ['T0']
    admission = s.script
    s = FixtureScript('invalid-join')
    s.step()
    assert s.state['tasks']['T0']['resume'] == ['error', 'invalid-join:S0']
    s.step()
    s.drain()
    assert s.view()['terminal'] == {
        'kind': 'value',
        'value': 'invalid-caught',
        'cause': None,
        'cancel': None,
        'failures': [],
        'children': [],
    }
    assert ['caught-invalid', 'invalid-join:S0'] in s.view()['events']
    invalid = s.script
    s = FixtureScript('deep')
    s.step()
    s.settle()
    s.step()
    s.step()
    s.settle(value='ACCEPTED-DEEP')
    s.do('cancel', scope='S2', reason=REASON)
    s.step()  # inner exits; middle enters drain, no body request yet
    assert s.state['tasks']['T0']['cancel_pending']
    assert s.state['tasks']['T0']['phase'] == 'drain'
    s.step()
    s.settle(value='ACCEPTED-MIDDLE')
    s.cancel('repeat')
    s.step()  # middle exits; outer body operation becomes cancellation boundary
    assert s.state['tasks']['T0']['resume'] == ['error', REASON]
    s.drain()
    assert s.view()['terminal'] == CANCELLED
    assert [e[1]['payload'] for e in s.view()['events'] if e[0] == 'pause'] == [
        ['gate'],
        ['inner', 'I'],
        ['middle', 'M'],
        ['parent-arbitrary', 'P'],
    ]
    return [admission, invalid, s.script]


def guest_source(fixture=False):
    """Assemble the unchanged real library plus optional bounded test body."""
    return (
        (ROOT / 'combined11_guest.py').read_text()
        + (ASYNC if fixture else '')
        + '\ndef adapt(body):\n    return body\n'
        + (ROOT / 'combined11_driver.py').read_text()
        + c.BRIDGE
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    report = {
        'status': 'FAIL',
        'cases': {},
        'runtime_sha256': scope9.runtime_identity(),
        'source_sha256': source_identity(),
        'inputs': {p.name: digest(p) for p in [*ROOT.glob('combined11*.py'), Path(__file__)]},
    }
    with args.report.open('x') as output:
        try:
            scripts = (
                [nested(stage, children) for children in (False, True) for stage in ('drain', 'queued-cleanup')]
                + [original_error()]
                + fixtures()
            )
            receipt = json.loads((ROOT / 'combined12_artifacts.json').read_text())
            assert receipt['runtime_sha256'] == report['runtime_sha256']
            assert receipt['source_sha256'] == report['source_sha256']
            for path, sha256 in receipt['combined_inputs'].items():
                assert digest(Path(path)) == sha256
            for backend in ('native', 'wasm'):
                assert {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)} == receipt[backend][
                    'artifacts'
                ]
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in scripts:
                    scope9.guest_source = lambda fixture=script.get('fixture', False): guest_source(fixture)
                    path = args.report.parent / (args.report.stem + '-cuts') / backend / script['name']
                    report['cases'].setdefault(script['name'], {})[backend] = c.run(command, script, path)
                    print(backend, script['name'], 'PASS', flush=True)
            report['status'] = 'PASS'
        except (AssertionError, ValueError, OSError) as error:
            report['error'] = str(error)
            print('FAIL', error, flush=True)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
