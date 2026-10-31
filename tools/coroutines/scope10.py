"""Run unchanged scope9 scripts with explicit async bodies and the accepted driver."""

import argparse
import json
import types
from pathlib import Path
from trace import digest, finite_dumps

import scope9
from async10 import DIRECTIVE
from verify import BASELINE, REPO, artifacts, source_identity

ROOT = Path(__file__).resolve().parent


def guest_source():
    text = scope9_source()
    bodies = (ROOT / 'scope10_bodies.py').read_text()
    return DIRECTIVE + text.replace('import json\n\nconfig =', bodies + '\nimport json\n\nconfig =')


def check_cpython(script):
    """Drive real Python async bodies via explicit await iterators, with no loop."""
    ns = {}
    exec(compile((ROOT / 'scope7_guest.py').read_text(), 'scope7_guest.py', 'exec'), ns)
    exec(compile((ROOT / 'scope10_bodies.py').read_text(), 'scope10_bodies.py', 'exec'), ns)
    ns['request'] = types.coroutine(ns['request'])
    parent, child = ns['parent_body'], ns['child_body']

    def continuation(function, task, mode):
        return (yield from function(task, mode).__await__())

    ns['parent_body'] = lambda task, mode: continuation(parent, task, mode)
    ns['child_body'] = lambda task, mode: continuation(child, task, mode)
    state = ns['new_driver'](script['mode'])
    assert ns['view'](state) == script['initial']
    for message, before, expected in script['turns']:
        assert ns['view'](state) == before
        if message['kind'] == 'tamper-shadow':
            state['tasks']['T0']['cancel'] = None
            state['tasks']['T0']['cause'] = None
        else:
            ns['command'](state, message)
        assert json.loads(finite_dumps(ns['view'](state))) == expected
    # Retain logical-disposal heaps; CPython reference loss is not VM disposal.
    RETAINED.append(state)


RETAINED = []
scope9_source = scope9.guest_source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    scope9.guest_source = guest_source
    report = {
        'status': 'FAIL',
        'source_sha256': source_identity(),
        'runtime_sha256': scope9.runtime_identity(),
        'artifacts': {b: {str(p.relative_to(REPO)): digest(p) for p in artifacts(b)} for b in ('native', 'wasm')},
        'cases': {},
    }
    with args.report.open('x') as output:
        try:
            scripts = scope9.scripts()
            for script in scripts:
                check_cpython(script)
            report['cpython_cases'] = len(scripts)
            for backend in ('native', 'wasm'):
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in scripts:
                    directory = args.report.parent / (args.report.stem + '-cuts') / backend / script['name']
                    report['cases'].setdefault(script['name'], {})[backend] = scope9.run(command, script, directory, 30)
                    print(backend, script['name'], 'PASS', flush=True)
            report['status'] = 'PASS'
        except (AssertionError, ValueError, OSError) as error:
            report['error'] = str(error)
            print('FAIL', error, flush=True)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
