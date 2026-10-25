"""Literal fresh-scope admission regressions with original/restored backend cuts."""

import argparse
import json
from pathlib import Path
from trace import digest, finite_dumps

import combined11 as c
import scope9
from combined13_bodies import ASYNC, PROJECTION, REVIEW
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent


class AdmissionScript(c.Script):
    """Execute actual CPython async syntax; literal assertions below own policy."""

    def __init__(self, mode, cleanup_error=False):
        self.name = 'repair13-' + mode + ('-cleanup-error' if cleanup_error else '')
        ns = c.namespace(True)
        exec(compile(REVIEW if mode == 'review' else ASYNC, 'repair13_body', 'exec'), ns)
        exec(compile(PROJECTION, 'repair13_projection', 'exec'), ns)
        parent = ns['parent_body']

        def lazy(state, task, mode):
            return (yield from parent(state, task, mode).__await__())

        ns['parent_body'] = lazy
        self.ns = [ns]
        self.states = [ns['new_driver'](mode)]
        c.RETAINED.extend(self.states)
        self.script = {'name': self.name, 'mode': mode, 'initial': self.view(), 'turns': []}

    def view(self):
        return json.loads(finite_dumps(self.ns[0]['view'](self.state)))


def reject_host(s, scope):
    """Host rejection adds only its receipt: no queue, ID, effect or advancement."""
    before = s.view()
    s.do('spawn', scope=scope)
    after = s.view()
    assert after == {**before, 'events': [*before['events'], ['reject-spawn', scope]]}


def scenario(mode, cleanup_error=False):
    """Review's deferred sequence plus immediate cancellation and open controls."""
    s = AdmissionScript(mode, cleanup_error)
    s.step()
    if mode != 'immediate':
        s.settle(value='GATE')
        s.step()  # inner drain
        s.step()  # inner cleanup
        s.settle(
            value='INNER-FAIL' if cleanup_error else 'INNER-ACCEPTED', outcome='error' if cleanup_error else 'value'
        )
        if mode != 'control':
            s.do('cancel', scope='S1', reason='FIRST')
            s.cancel('REPEAT-BEFORE-RETURN')
        assert s.state['tasks']['T0']['resume'] == [
            'error' if cleanup_error else 'value',
            'INNER-FAIL' if cleanup_error else 'INNER-ACCEPTED',
        ]
        s.step()  # accepted inner reply/exit before first outer-body delivery
    if mode == 'control':
        s.settle(value='AFTER-INNER')
    else:
        if mode == 'immediate':
            s.cancel('FIRST')
        s.cancel('REPEAT-QUEUED')
        assert s.state['tasks']['T0']['resume'] == ['error', 'FIRST']
    next_scope = 'S1' if mode == 'immediate' else 'S2'
    counters = [s.state['next_task'], s.state['next_boundary']]
    s.step()  # catches cancellation and enters fresh context; first spawn request
    assert s.state['scopes'][next_scope]['phase'] == 'body'
    if mode == 'control':
        assert s.state['tasks']['T0']['resume'] == ['value', 'T1']
        s.do('spawn', scope=next_scope)  # normal host admission remains lazy
        assert list(s.state['tasks']) == ['T0', 'T1', 'T2']
        assert not s.state['tasks']['T1']['started']
        assert not s.state['tasks']['T2']['started']
        s.step()  # second continuation spawn
        assert s.state['tasks']['T0']['resume'] == ['value', 'T3']
        s.step()  # fresh body effect
        # A valid child actually acquires and uses before its owning scope closes.
        s.step('T1')
        s.settle('T1', 'HANDLE')
        s.step('T1')
        assert s.state['tasks']['T1']['wait']['payload'] == ['use', 'HANDLE']
    else:
        assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:' + next_scope]
        assert [s.state['next_task'], s.state['next_boundary']] == counters
        assert list(s.state['tasks']) == ['T0']
        reject_host(s, next_scope)
        reject_host(s, next_scope)
        s.cancel('REPEAT-REJECTED')
        assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:' + next_scope]
        s.step()  # catches first rejection and requests second spawn
        assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:' + next_scope]
        assert [s.state['next_task'], s.state['next_boundary']] == counters
        reject_host(s, next_scope)
        s.step()  # catches second rejection and continues fresh body
        assert s.state['tasks']['T0']['wait']['payload'] == ['fresh-body']
        reject_host(s, next_scope)
        assert s.state['scopes'][next_scope]['children'] == []
    s.settle(value='FRESH-DONE')
    s.step()  # fresh scope drain
    if mode != 'control':
        reject_host(s, next_scope)
    s.drain()
    expected_payloads = [['after-inner']] if mode == 'immediate' else [['gate'], ['inner-last', 'I']]
    if mode == 'control':
        expected_payloads += [
            ['after-inner'],
            ['fresh-body'],
            ['acquire', 'fresh'],
            ['use', 'HANDLE'],
            ['another-name', 'HANDLE'],
            ['arbitrary-child', 'HANDLE'],
        ]
    else:
        expected_payloads += [['fresh-body']]
    expected_payloads += [['fresh-last', 'F'], ['outer-last', 'P']]
    assert [e[1]['payload'] for e in s.view()['events'] if e[0] == 'pause'] == expected_payloads
    assert s.view()['terminal'] == {
        'kind': 'value' if mode == 'control' else 'cancelled',
        'value': 'returned',
        'cause': None if mode == 'control' else 'FIRST',
        'cancel': None if mode == 'control' else 'FIRST',
        'failures': ['INNER-FAIL'] if cleanup_error else [],
        'children': [],
    }
    if mode != 'control':
        assert [e for e in s.view()['events'] if e[0].startswith('caught')] == [
            ['caught', 'FIRST'],
            ['caught-spawn', 'scope-closed:' + next_scope],
            ['caught-spawn', 'scope-closed:' + next_scope],
        ]
        assert list(s.state['tasks']) == ['T0']
        assert [e for e in s.view()['events'] if e[0] == 'reject-spawn'] == [
            ['reject-spawn', next_scope] for _ in range(7)
        ]
    s.script['literal_payloads'] = expected_payloads
    return s.script


def review_sequence():
    """The review body/host sequence now rejects before any child can acquire/use."""
    s = AdmissionScript('review')
    s.step()
    s.settle(value='GATE')
    s.step()
    s.step()
    s.settle(value='INNER-ACCEPTED')
    s.do('cancel', scope='S1', reason='FIRST')
    s.cancel('REPEAT-BEFORE-RETURN')
    assert s.state['tasks']['T0']['resume'] == ['value', 'INNER-ACCEPTED']
    s.step()
    assert s.state['tasks']['T0']['resume'] == ['error', 'FIRST']
    s.cancel('REPEAT-QUEUED')
    s.step()
    assert s.state['tasks']['T0']['resume'] == ['error', 'scope-closed:S2']
    assert s.state['next_task'] == 1
    assert s.state['next_boundary'] == 2
    reject_host(s, 'S2')
    s.step()  # unhandled admission error enters fresh drain, then outer unwind
    s.drain()
    assert s.view()['terminal'] == {
        'kind': 'cancelled',
        'value': None,
        'cause': 'scope-closed:S2',
        'cancel': 'FIRST',
        'failures': [],
        'children': [],
    }
    assert [e[1]['payload'] for e in s.view()['events'] if e[0] == 'pause'] == [
        ['gate'],
        ['inner-last', 'I'],
        ['fresh-last', 'F'],
        ['outer-last', 'P'],
    ]
    assert list(s.state['tasks']) == ['T0']
    return s.script


def guest_source(review=False):
    """Retain the fixed factory and library, changing only this test's parent body."""
    return (
        (ROOT / 'combined11_guest.py').read_text()
        + (REVIEW if review else ASYNC)
        + '\ndef adapt(body):\n    return body\n'
        + (ROOT / 'combined11_driver.py').read_text()
        + PROJECTION
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
        'inputs': {p.name: digest(p) for p in [*ROOT.glob('combined11*.py'), *ROOT.glob('combined13*.py')]},
    }
    with args.report.open('x') as output:
        try:
            scripts = [
                review_sequence(),
                scenario('deferred'),
                scenario('immediate'),
                scenario('deferred', True),
                scenario('control'),
            ]
            receipt = json.loads((ROOT / 'combined13_artifacts.json').read_text())
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
                    scope9.guest_source = lambda review=script['mode'] == 'review': guest_source(review)
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
