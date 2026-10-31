"""Literal policy checks + CPython syntax oracle + full native/Wasm replay."""

import argparse
import hashlib
import json
import shutil
import subprocess
import types
from pathlib import Path
from trace import digest, event_from, execute, finite_dumps

from verify import BASELINE, REPO, artifacts

ROOT = Path(__file__).resolve().parent
FILES = ('bindings.py', 'kernel.py', 'coordinator.py', 'workload.py')
DIRECTIVE = 'from __monty_host__ import host_coroutines\n'
BRIDGE = """
import json
config = json.loads(exchange(json.dumps({'configure': 'coordinator1'})))
state = new_driver(config['mode'])
turn(state, [], 4)
while state['tasks']['T0']['result'] is None:
    messages = json.loads(exchange(json.dumps(observe(state))))
    turn(state, messages, 4)
json.dumps(observe(state))
"""


def source():
    return '\n'.join((ROOT / name).read_text() for name in FILES) + '\ndef adapt(body):\n    return body\n' + BRIDGE


def reference(mode):
    ns = {}
    for name in FILES:
        exec(compile((ROOT / name).read_text().replace(DIRECTIVE, ''), name, 'exec'), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda body: body.__await__()
    state = ns['new_driver'](mode)
    ns['turn'](state, [], 4)
    return ns, state


def detached(value):
    return json.loads(finite_dumps(value))


def script(mode):
    ns, state = reference(mode)
    initial = detached(ns['observe'](state))
    views = [initial]
    inputs = [{'mode': mode}]
    serial = 0
    cancelled = False
    failed = False
    repeated = False
    noisy = False

    def event(kind, **fields):
        nonlocal serial
        serial += 1
        return {'run': 'C1', 'event': serial, 'kind': kind, **fields}

    for _ in range(120):
        before = views[-1]
        if before['terminal'] is not None:
            break
        messages = []
        pending = before['pending']
        # Run ready work first; host never chooses a continuation.
        if (
            mode == 'host-wake-cancel'
            and not cancelled
            and any(t in before['ready'] for t in before['queues']['work']['put'])
        ):
            messages = [event('cancel', reason='')]
            cancelled = True
        elif not before['ready']:
            reads = [p for p in pending if p['payload'][0] == 'read']
            if mode in ('host-cancel', 'host-cancel-cleanup-error') and reads and not cancelled:
                messages = [event('cancel', reason='')]
                cancelled = True
            else:
                # The gate wins before a queued leaf reply, proving that the
                # supplied guest cancel can discard an accepted delivery.
                gates = [p for p in pending if p['payload'][0] == 'cancel-gate']
                ordered = gates + [p for p in reversed(pending) if p not in gates]
                if reads and mode in ('success', 'host-error', 'cleanup-error'):
                    ordered = [reads[-1]]
                for p in ordered:
                    outcome = 'value'
                    value = 'ok:' + ':'.join(str(x) for x in p['payload'])
                    if mode == 'host-error' and p['payload'][0] == 'read' and not failed:
                        outcome, value, failed = 'error', 'host-original', True
                    if (
                        mode in ('cleanup-error', 'host-cancel-cleanup-error')
                        and p['payload'][0] == 'audit'
                        and not failed
                    ):
                        outcome, value, failed = 'error', 'audit-original', True
                    messages.append(event('reply', operation=p['boundary'], outcome=outcome, value=value))
                if (
                    mode in ('host-cancel', 'host-wake-cancel', 'host-cancel-cleanup-error')
                    and cancelled
                    and not repeated
                ):
                    messages.insert(0, event('cancel', reason='REPEAT'))
                    repeated = True
                if mode == 'success' and messages and not noisy:
                    messages = [
                        event('reply', operation=messages[0]['operation'], outcome='unsupported', value=None),
                        event('advance', task='T0'),
                    ] + messages
                    messages += [
                        messages[2],
                        event('reply', operation='B999', outcome='value', value='late'),
                        {**event('reply', operation='B0', outcome='value', value='stale'), 'run': 'OLD'},
                    ]
                    noisy = True
        ns['turn'](state, messages, 4)
        inputs.append(messages)
        views.append(detached(ns['observe'](state)))
    assert views[-1]['terminal'] is not None, mode
    literal_checks(mode, views)
    return {'mode': mode, 'inputs': inputs, 'views': views}


def literal_checks(mode, views):
    end = views[-1]
    terminal = end['terminal']
    payloads = [e[1]['payload'] for e in end['events'] if e[0] == 'pause']
    assert payloads[-1] == ['close', 'parent'], (mode, payloads)
    assert all(t[1] == 'done' for t in end['tasks'])
    assert all(s[2] == 'exited' for s in end['scopes'])
    assert any(v['ready'] for v in views), 'missing voluntary budget snapshot'
    if mode == 'success':
        assert terminal['kind'] == 'value'
        assert [p for p in payloads if p[0] == 'read'] == [['read', 'A'], ['read', 'B'], ['read', 'C']]
        assert [e for e in end['events'] if e[0] == 'put'] == [
            ['put', 'T1', 'A'],
            ['put', 'T1', 'B'],
            ['put', 'T1', 'C'],
            ['put', 'T1', None],
            ['put', 'T1', None],
        ]
        assert [o['value'] for o in terminal['value']] == ['produced', ['ok:read:A'], ['ok:read:B', 'ok:read:C']]
        assert len([e for e in end['events'] if e[0] == 'reject']) == 5
        assert ['join-error', 'invalid-join:S0'] in end['events']
        assert any(len([p for p in v['pending'] if p['payload'][0] == 'read']) == 2 for v in views)
        assert any(v['ready'] and any(q['get'] or q['put'] for q in v['queues'].values()) for v in views)
    elif mode in ('host-cancel', 'host-wake-cancel', 'host-cancel-cleanup-error'):
        assert terminal['kind'] == 'cancelled' and terminal['cancel'] == '' and terminal['value'] == 'caught-host'
        assert end['host_cancelled'] and end['host_reason'] == ''
        assert any(e[0] == 'admission-closed' for e in end['events'])
        assert not any(e[0] == 'task' and e[2] is not None and e[1] == 'T4' for e in end['events'])
        if mode == 'host-cancel-cleanup-error':
            assert any(t[6]['failures'] == ['audit-original'] for t in end['tasks'])
        if mode == 'host-wake-cancel':
            assert [e for e in end['events'] if e[0] == 'put'] == [['put', 'T1', 'A']]
    elif mode == 'body-error':
        assert terminal['kind'] == 'error' and terminal['cause'] == 'body-original'
    elif mode == 'host-error':
        assert terminal['kind'] == 'error' and terminal['cause'] == 'joined-child-error'
        assert any(t[6]['cause'] == 'host-original' for t in end['tasks'])
    elif mode == 'cleanup-error':
        assert terminal['kind'] == 'error'
        assert any(t[6]['failures'] == ['audit-original'] for t in end['tasks'])
        assert ['close', 'worker-a'] in payloads and ['close', 'worker-b'] in payloads
    elif mode == 'early':
        assert terminal['value'] == 'early'
    else:
        assert not end['host_cancelled'] and terminal['kind'] == 'value'
        outcome = terminal['value']
        assert outcome['kind'] == ('value' if mode == 'suppress' else 'cancelled'), (mode, outcome)
        if mode == 'suppress':
            assert outcome['value'] == 'suppressed-task-cancel'
        else:
            assert outcome['cause'] == 'task-cancel:explicit'
        assert payloads[-2:] == [['close', 'inner'], ['close', 'parent']]
        if mode == 'join':
            assert payloads.index(['close', 'grandchild']) < payloads.index(['close', 'victim'])
        if mode in ('put', 'get'):
            assert not any(e[0] in ('put', 'get') and e[1] == 'T1' for e in end['events'])


def identity():
    receipt = json.loads((ROOT.parent / 'combined13_artifacts.json').read_text())
    names = (
        subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=REPO)
        .decode()
        .splitlines()
    )
    runtime = {
        name: digest(REPO / name)
        for name in sorted(set(names))
        if (name.startswith('crates/') and ('/src/' in name or name.endswith('Cargo.toml')))
        or name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
    }
    hashed = hashlib.sha256(finite_dumps(runtime, sort_keys=True).encode()).hexdigest()
    assert hashed == receipt['runtime_sha256'], 'runtime changed: artifacts stale'
    hashes = {b: {str(p.relative_to(REPO)): digest(p) for p in artifacts(b)} for b in ('native', 'wasm')}
    assert all(hashes[b] == receipt[b]['artifacts'] for b in hashes), 'artifact mismatch'
    return {'runtime': hashed, 'artifacts': hashes, 'inputs': {p.name: digest(p) for p in ROOT.glob('*.py')}}


def replay(backend, case, directory):
    directory.mkdir(parents=True, exist_ok=False)
    guest = directory / 'guest.py'
    guest.write_text(source())
    command = (
        [str(BASELINE / 'target/debug/coroutine-baseline')]
        if backend == 'native'
        else ['node', str(BASELINE / 'wasm.mjs')]
    )
    evidence = []

    def invoke(args):
        raw = execute([*command, *args], 30)
        evidence.append({'args': args, **raw})
        return event_from(raw)

    snapshot = directory / 'cut-0.bin'
    assert invoke(['start', str(guest), str(snapshot)]) == {'effect': {'configure': 'coordinator1'}}
    for i, (message, expected) in enumerate(zip(case['inputs'], case['views'], strict=True)):
        evidence[-1]['snapshot_sha256'] = digest(snapshot)
        assert invoke(['dispose', str(snapshot)]) == {'disposed': True}
        target = directory / f'cut-{i + 1}.bin'
        actual = invoke(['resume', str(snapshot), finite_dumps(message), str(target)])
        key = 'complete' if expected['terminal'] is not None else 'effect'
        assert actual == {key: expected}, (backend, case['mode'], i, actual, expected)
        # Independently restore the SAME input bytes: no live host continuation,
        # no shared in-memory guest graph, full deterministic observation check.
        if i in (0, 2, len(case['inputs']) - 2):
            branch = directory / f'branch-{i}.bin'
            assert invoke(['resume', str(snapshot), finite_dumps(message), str(branch)]) == actual
            if key == 'effect':
                assert invoke(['dispose', str(branch)]) == {'disposed': True}
        if case['mode'] == 'success' and i == 2:
            ns, state = reference('success')
            ns['turn'](state, case['inputs'][1], 4)
            cancellation = [{'run': 'C1', 'event': 'branch-only', 'kind': 'cancel', 'reason': 'branch'}]
            ns['turn'](state, cancellation, 4)
            wanted = detached(ns['observe'](state))
            original_hash = digest(snapshot)
            fork = directory / 'divergent.bin'
            assert invoke(['resume', str(snapshot), finite_dumps(cancellation), str(fork)]) == {'effect': wanted}
            assert invoke(['dispose', str(fork)]) == {'disposed': True}
            assert digest(snapshot) == original_hash
            independent = directory / 'independent.bin'
            assert invoke(['resume', str(snapshot), finite_dumps(message), str(independent)]) == actual
            assert invoke(['dispose', str(independent)]) == {'disposed': True}
        snapshot = target
    return {'status': 'PASS', 'cuts': len(case['inputs']), 'evidence': evidence}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--reference-only', action='store_true')
    args = parser.parse_args()
    cases = [
        script(mode)
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
        )
    ]
    if args.reference_only:
        print([(c['mode'], len(c['inputs'])) for c in cases])
        return
    assert args.report is not None
    assert shutil.disk_usage(REPO).free >= 10 * 1024**3
    with args.report.open('x') as output:
        report = {'status': 'FAIL', 'identity': identity(), 'cases': cases, 'backends': {}}
        try:
            for backend in ('native', 'wasm'):
                report['backends'][backend] = {}
                for case in cases:
                    directory = args.report.parent / (args.report.stem + '-cuts') / backend / case['mode']
                    report['backends'][backend][case['mode']] = replay(backend, case, directory)
                    print(backend, case['mode'], 'PASS', flush=True)
            report['status'] = 'PASS'
        finally:
            output.write(finite_dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
