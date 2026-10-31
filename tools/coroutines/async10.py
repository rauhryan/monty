"""Async syntax/explicit-awaitable/generator equivalence on fresh native and Wasm instances."""

import argparse
import json
import types
from pathlib import Path
from trace import digest, event_from, execute, finite_dumps

import async10_reference
import scope9
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent
DIRECTIVE = 'from __monty_host__ import host_coroutines\n'
BRIDGE = """
import json
config = json.loads(exchange(json.dumps({'configure': 10})))
state = new_state(config['mode'])
while state['terminal'] is None:
    message = json.loads(exchange(json.dumps(observe(state))))
    apply(state, message)
json.dumps(observe(state))
"""


def guest_source():
    return (
        (ROOT / 'async10_guest.py').read_text()
        + '\ndef adapt(body):\n    return body\n'
        + (ROOT / 'async10_driver.py').read_text()
        + BRIDGE
    )


def namespace(syntax):
    ns = {}
    if syntax:
        # Only remove the Monty compiler directive. Python parses and executes
        # the original async definitions/async with. No event loop is involved.
        exec(compile((ROOT / 'async10_guest.py').read_text().replace(DIRECTIVE, '', 1), 'async10_guest.py', 'exec'), ns)
        ns['effect'] = types.coroutine(ns['effect'])
        ns['adapt'] = lambda body: body.__await__()
    else:
        ns = {'cart': async10_reference.cart, 'adapt': lambda body: body}
    exec(compile((ROOT / 'async10_driver.py').read_text(), 'async10_driver.py', 'exec'), ns)
    return ns


def scenarios():
    result = []
    advance = {'kind': 'advance'}

    def settle(value=None, outcome='value'):
        return {'kind': 'settle', 'outcome': outcome, 'value': value}

    for mode in (
        'normal',
        'return',
        'error',
        'suppress',
        'cancel',
        'catch',
        'entry-error',
        'acquire-error',
        'cleanup-error',
    ):
        commands = [advance, settle('entry-failed', 'error') if mode == 'entry-error' else settle(), advance]
        if mode != 'entry-error':
            commands += [settle('acquire-failed', 'error') if mode == 'acquire-error' else settle('H'), advance]
            if mode in ('normal', 'cancel', 'catch', 'cleanup-error'):
                commands += [
                    {'kind': 'cancel', 'value': 'alarm'} if mode in ('cancel', 'catch') else settle('used'),
                    advance,
                ]
            if mode != 'acquire-error':
                if mode in ('cancel', 'catch'):
                    commands += [{'kind': 'cancel', 'value': 'repeat'}]
                commands += [
                    settle() if mode in ('normal', 'return') else settle('audit-failed', 'error'),
                    advance,
                    settle(),
                    advance,
                ]
        states = []
        for syntax in (False, True):
            ns = namespace(syntax)
            state = ns['new_state'](mode)
            views = [json.loads(finite_dumps(ns['observe'](state)))]
            for command in commands:
                ns['apply'](state, command)
                views.append(json.loads(finite_dumps(ns['observe'](state))))
            states.append(views)
        assert states[0] == states[1], mode
        end = states[0][-1]
        expected = {
            'normal': {'kind': 'value', 'value': 'used', 'cause': None},
            'return': {'kind': 'value', 'value': 'H', 'cause': None},
            'error': {'kind': 'error', 'value': None, 'cause': 'body-failed'},
            'suppress': {'kind': 'error', 'value': 'suppressed', 'cause': None},
            'cancel': {'kind': 'cancelled', 'value': None, 'cause': 'alarm'},
            'catch': {'kind': 'cancelled', 'value': 'caught', 'cause': 'alarm'},
            'entry-error': {'kind': 'error', 'value': None, 'cause': 'entry-failed'},
            'acquire-error': {'kind': 'error', 'value': None, 'cause': 'acquire-failed'},
            'cleanup-error': {'kind': 'error', 'value': 'used', 'cause': None},
        }[mode]
        assert end['terminal'] == expected, (mode, end)
        assert len(end['events']) == (1 if mode == 'entry-error' else 2)
        assert end['failures'] == (
            [] if mode in ('normal', 'return', 'entry-error', 'acquire-error') else ['audit-failed']
        )
        result.append({'mode': mode, 'commands': commands, 'views': states[0]})
    return result


def run(command, script, directory):
    directory.mkdir(parents=True, exist_ok=False)
    guest = directory / 'guest.py'
    guest.write_text(guest_source())
    snapshot = directory / 'cut-0.bin'
    evidence = []
    host_cancel = None

    def invoke(argv):
        raw = execute(argv, 30)
        evidence.append({'command': argv, **raw})
        return event_from(raw)

    event = invoke([*command, 'start', str(guest), str(snapshot)])
    assert event == {'effect': {'configure': 10}}, event
    replies = [{'mode': script['mode']}, *script['commands']]
    for index, (message, view) in enumerate(zip(replies, script['views'], strict=True)):
        evidence[-1]['snapshot_sha256'] = digest(snapshot)
        assert invoke([*command, 'dispose', str(snapshot)]) == {'disposed': True}
        if message.get('kind') == 'cancel' and host_cancel is None:
            host_cancel = message['value']
        next_snapshot = directory / f'cut-{index + 1}.bin'
        event = invoke([*command, 'resume', str(snapshot), finite_dumps(message), str(next_snapshot)])
        key = 'complete' if view['terminal'] is not None else 'effect'
        assert event == {key: view}, (script['mode'], index, event, view)
        snapshot = next_snapshot
    assert 'complete' in event
    if host_cancel is not None:
        assert event['complete']['terminal']['kind'] == 'cancelled'
    return {
        'status': 'PASS',
        'cuts': len(replies),
        'host_cancel': host_cancel,
        'host_outcome': 'cancelled' if host_cancel is not None else event['complete']['terminal']['kind'],
        'evidence': evidence,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    report = {
        'status': 'FAIL',
        'source_sha256': source_identity(),
        'runtime_sha256': scope9.runtime_identity(),
        'artifacts': {b: {str(p.relative_to(REPO)): digest(p) for p in artifacts(b)} for b in ('native', 'wasm')},
        'cases': {},
    }
    with args.report.open('x') as output:
        try:
            scripts = scenarios()
            for backend in ('native', 'wasm'):
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in scripts:
                    directory = args.report.parent / (args.report.stem + '-cuts') / backend / script['mode']
                    report['cases'].setdefault(script['mode'], {})[backend] = run(command, script, directory)
                    print(backend, script['mode'], 'PASS', flush=True)
            report['status'] = 'PASS'
        except (AssertionError, ValueError, OSError) as error:
            report['error'] = str(error)
            print('FAIL', error, flush=True)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
