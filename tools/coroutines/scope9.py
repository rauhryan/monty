"""Actual aggregate child-scope proof; fixed scope7/repair8 oracle, fresh process at every exchange."""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from trace import digest, event_from, execute, finite_dumps

import scope7_reference as reference
from scope7_guest import new_driver, view
from scope7_probe import source
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent
ACCEPTED = '355990c6c98fe187e76fe4bc459569dbeb00c86b82c1d35b444ff94abc0de795'


def runtime_identity():
    """Hash runtime source separately from new native tests, never call test edits a rebuild."""
    names = (
        subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=REPO)
        .decode()
        .splitlines()
    )
    paths = [
        REPO / name
        for name in sorted(set(names))
        if ('/src/' in name and name.startswith('crates/'))
        or name.endswith('Cargo.toml')
        or name in ('Cargo.lock', 'rust-toolchain.toml')
    ]
    return hashlib.sha256(
        finite_dumps({str(p.relative_to(REPO)): digest(p) for p in paths}, sort_keys=True).encode()
    ).hexdigest()


GRAPH_CHECK = """
def check_graph(state):
    for task_id in state['tasks']:
        task = state['tasks'][task_id]
        assert task['id'] == task_id
        if task['pending_join'] is not None:
            child = state['tasks'][task['pending_join']]
            assert task['resume'][1] is child['result']
            assert not child['consumed']
        if task['wait'] is not None:
            found = False
            for event in state['events']:
                if event[0] == 'pause' and event[1] is task['wait']:
                    found = True
            assert found
        if task['result'] is not None:
            assert task['result']['failures'] is task['failures']
        if task['phase'] == 'cleanup':
            for child in task['children']:
                assert state['tasks'][child]['result'] is not None
"""


def guest_source():
    """Check graph identities after every restore/command without exporting generators."""
    text = source()
    text = text.replace('import json\n\nconfig =', GRAPH_CHECK + '\nimport json\n\nconfig =')
    text = text.replace(
        '    message = json.loads(exchange(json.dumps(view(state))))',
        '    check_graph(state)\n    message = json.loads(exchange(json.dumps(view(state))))',
    )
    # Deliberately cooperative metadata can be changed by guest code. This
    # test-only branch is an adversarial authority probe, not a library API.
    return text.replace(
        '    command(state, message)',
        "    if message['kind'] == 'tamper-shadow':\n        state['tasks']['T0']['cancel'] = None\n        state['tasks']['T0']['cause'] = None\n    else:\n        command(state, message)",
    )


def scripts():
    """Record exact host commands and detached observations from the fixed reference."""
    recordings = []
    current = None
    original = reference.act

    def record(state, kind, task='T0', **fields):
        nonlocal current
        if current is not state:
            recordings.append({'mode': state['mode'], 'initial': json.loads(json.dumps(view(state))), 'turns': []})
            current = state
        before = json.loads(json.dumps(view(state)))
        result = original(state, kind, task, **fields)
        recordings[-1]['turns'].append(({'run': 'R7', 'kind': kind, 'task': task, **fields}, before, result))
        return result

    reference.act = record
    try:
        states = reference.scenarios()
    finally:
        reference.act = original
    expected = json.loads((ROOT / 'scope7_expected.json').read_text())
    assert len(states) == len(recordings) == 16
    for (name, state), script in zip(states.items(), recordings, strict=True):
        assert reference.exact_trace(state) == expected[name], name
        script['name'] = name
    caught = next(script for script in recordings if script['name'] == 'caught_success_sticky_cancel')
    recordings.append(mutation_script(caught))
    recordings.append(acquisition_script())
    return recordings


def acquisition_script():
    """Extra gate with literal assertions: external reconciliation is not guest cleanup."""
    state = new_driver('normal')
    script = {
        'name': 'accepted_acquire_cancel_before_delivery',
        'mode': 'normal',
        'initial': json.loads(json.dumps(view(state))),
        'turns': [],
    }

    def do(kind, task='T0', **fields):
        """Keep before/after views for exact differential comparison on both VMs."""
        before = json.loads(json.dumps(view(state)))
        after = reference.act(state, kind, task, **fields)
        script['turns'].append(({'run': 'R7', 'kind': kind, 'task': task, **fields}, before, after))

    for _ in range(3):
        do('advance')
    do('advance', 'T1')
    boundary = state['tasks']['T1']['wait']['boundary']
    do('settle', 'T1', boundary=boundary, outcome='value', value='R8-QUEUED-HANDLE')
    assert state['tasks']['T1']['resume'] == ['value', 'R8-QUEUED-HANDLE']
    do('cancel', scope='S0', reason='acquire-race')
    do('advance')
    for child in ['T1', 'T2']:
        do('advance', child)
        do('advance', child)
    do('advance')
    do('settle', boundary=state['tasks']['T0']['wait']['boundary'], outcome='value', value=None)
    do('advance')
    assert state['tasks']['T0']['result'] == {
        'kind': 'cancelled',
        'value': None,
        'cause': 'acquire-race',
        'cancel': 'acquire-race',
        'failures': [],
        'children': [],
    }
    assert [e[1]['payload'] for e in state['events'] if e[0] == 'pause'] == [['acquire', 'a'], ['release', 'parent-H']]
    return script


def mutation_script(caught):
    """Exercise an actual guest successful exit after clearing cancellation shadows."""
    state = new_driver('catch')
    script = {
        'name': 'authority_veto_guest_metadata_mutation',
        'mode': 'catch',
        'initial': json.loads(json.dumps(view(state))),
        'turns': [],
    }
    messages = [turn[0] for turn in caught['turns']]
    messages.insert(-1, {'run': 'R7', 'kind': 'tamper-shadow', 'task': 'T0'})
    for message in messages:
        before = json.loads(json.dumps(view(state)))
        if message['kind'] == 'tamper-shadow':
            state['tasks']['T0']['cancel'] = None
            state['tasks']['T0']['cause'] = None
            after = json.loads(json.dumps(view(state)))
        else:
            fields = {k: v for k, v in message.items() if k not in ('run', 'kind', 'task')}
            after = reference.act(state, message['kind'], message['task'], **fields)
        script['turns'].append((message, before, after))
    assert state['tasks']['T0']['result'] == {
        'kind': 'value',
        'value': 'caught-parent-success',
        'cause': None,
        'cancel': None,
        'failures': [],
        'children': [],
    }
    return script


class Authority:
    """Bounded scripted receipts and producer fence, external to the guest graph."""

    def __init__(self):
        """Start one bounded fixture run; there is no production persistence claim."""
        self.epoch = 0
        self.cancelled = {}
        self.receipts = []
        self.reconciled = []
        self.rejected_producers = 0

    def authorize(self, epoch):
        """Reject a superseded producer before any VM advancement."""
        if epoch != self.epoch:
            self.rejected_producers += 1
            return False
        return True

    def restored(self):
        """Fence every prior process and explicitly test its rejection."""
        previous = self.epoch
        self.epoch += 1
        assert not self.authorize(previous)
        assert self.authorize(self.epoch)

    def observe(self, message, before, after):
        """Retain accepted receipts and classify undelivered acquisition orphans."""
        new_events = after['events'][len(before['events']) :]
        if message['kind'] == 'settle' and any(e[0] == 'accept' for e in new_events):
            descriptor = next(p for p in before['pending'] if p['task'] == message['task'])
            self.receipts.append(
                {'operation': descriptor, 'outcome': message['outcome'], 'value': message['value'], 'delivered': False}
            )
        if message['kind'] == 'cancel' and any(e[0] == 'cancel' for e in new_events):
            for event in new_events:
                if event[0] == 'cancel':
                    self.cancelled.setdefault(event[1], event[2])
                    for receipt in self.receipts:
                        if receipt['operation']['task'] == event[1] and not receipt['delivered']:
                            if receipt['operation']['payload'][0] == 'acquire' and receipt['outcome'] == 'value':
                                if receipt not in self.reconciled:
                                    self.reconciled.append(receipt)
        if message['kind'] == 'advance':
            for receipt in self.receipts:
                if receipt['operation']['task'] == message['task'] and receipt not in self.reconciled:
                    receipt['delivered'] = True

    def outcome(self, terminal):
        """Apply host-owned cancellation even if the guest clears every shadow."""
        return 'cancelled' if 'T0' in self.cancelled else terminal['kind']


def run(command, script, directory, timeout):
    directory.mkdir(parents=True, exist_ok=False)
    guest = directory / 'guest.py'
    guest.write_text(guest_source())
    snapshot = directory / 'cut-0.bin'
    evidence = []
    authority = Authority()

    def invoke(argv):
        result = execute(argv, timeout)
        evidence.append({'command': argv, **result})
        return event_from(result)

    event = invoke([*command, 'start', str(guest), str(snapshot)])
    assert event == {'effect': {'kind': 'configure', 'contract': 7}}, event
    replies = [({'mode': script['mode']}, None, script['initial']), *script['turns']]
    disposed = False
    for index, (message, before, expected) in enumerate(replies):
        # Every original is dumped and physically dropped by the existing adapter.
        # Independently restore/drop the same aggregate without any guest command.
        assert snapshot.is_file()
        evidence[-1]['snapshot_sha256'] = digest(snapshot)
        assert invoke([*command, 'dispose', str(snapshot)]) == {'disposed': True}
        authority.restored()
        if message.get('kind') == 'dispose':
            # Logical reference disposal and its late reply are replaced by actual
            # physical drop. No send/throw/close or fake release is performed.
            assert event == {'effect': before}
            disposed = True
            break
        assert authority.authorize(authority.epoch)
        if message.get('kind') == 'cancel' and message['task'] == 'T0' and message['scope'] == 'S0':
            # Host accepts its scripted root intent BEFORE guest execution; no
            # guest acknowledgement/shadow is necessary for this sticky veto.
            authority.cancelled.setdefault('T0', message['reason'])
        next_snapshot = directory / f'cut-{index + 1}.bin'
        event = invoke([*command, 'resume', str(snapshot), finite_dumps(message), str(next_snapshot)])
        terminal = expected['terminal'] is not None
        assert event == {('complete' if terminal else 'effect'): expected}, (index, message, event, expected)
        if before is not None:
            authority.observe(message, before, expected)
        if terminal:
            assert index == len(replies) - 1
            if script['name'] == 'authority_veto_guest_metadata_mutation':
                assert expected['terminal']['kind'] == 'value'
                assert authority.outcome(expected['terminal']) == 'cancelled'
            else:
                assert authority.outcome(expected['terminal']) == expected['terminal']['kind']
            if 'T0' in authority.cancelled:
                assert authority.outcome({'kind': 'value', 'cancel': None}) == 'cancelled'
        snapshot = next_snapshot
    assert disposed or 'complete' in event
    if script['name'] == 'accepted_acquire_cancel_before_delivery':
        assert len(authority.reconciled) == 1
        assert authority.reconciled[0]['value'] == 'R8-QUEUED-HANDLE'
        assert not any(
            e[0] == 'pause' and e[1]['payload'] == ['release', 'R8-QUEUED-HANDLE'] for e in event['complete']['events']
        )
    return {
        'status': 'PASS',
        'exchanges': len(evidence) // 2,
        'physical_disposal': True,
        'guest_terminal': None if disposed else event['complete']['terminal'],
        'host_outcome': 'disposed' if disposed else authority.outcome(event['complete']['terminal']),
        'authority': vars(authority),
        'evidence': evidence,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--baseline', type=Path, default=ROOT / 'scope9_artifacts.json')
    args = parser.parse_args()
    with args.report.open('x') as output:
        report = {
            'status': 'FAIL',
            'cases': {},
            'runtime_sha256': runtime_identity(),
            'full_source_sha256': source_identity(),
        }
        try:
            baseline = json.loads(args.baseline.read_text())
            assert baseline['accepted_source_sha256'] == ACCEPTED
            assert baseline['runtime_sha256'] == report['runtime_sha256'], 'runtime source changed; rebuild required'
            report['association'] = baseline
            report['inputs'] = {
                name: digest(ROOT / name)
                for name in (
                    'scope9.py',
                    'scope7_guest.py',
                    'scope7_reference.py',
                    'scope7_probe.py',
                    'scope7_expected.json',
                )
            }
            selected = scripts()
            for backend in ('native', 'wasm'):
                hashes = {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)}
                assert hashes == baseline[backend]['artifacts'], 'artifact mismatch'
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in selected:
                    name = script['name']
                    directory = args.report.parent / (args.report.stem + '-cuts') / backend / name
                    try:
                        result = run(command, script, directory, args.timeout)
                    except (OSError, ValueError, AssertionError) as error:
                        result = {'status': 'FAIL', 'error': str(error)}
                    report['cases'].setdefault(name, {})[backend] = result
                    print(f'{backend:6} {name:48} {result["status"]}', flush=True)
            assert all(r[b]['status'] == 'PASS' for r in report['cases'].values() for b in ('native', 'wasm'))
            report['status'] = 'PASS'
        except (OSError, ValueError, AssertionError) as error:
            report['error'] = str(error)
            print(f'FAIL: {error}', flush=True)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
