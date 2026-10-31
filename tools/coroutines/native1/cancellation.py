"""Literal single-task cancellation probes; never import or rewrite the fixed oracle."""

import argparse
import json
from pathlib import Path

from adapter import execute

PRELUDE = """from __monty_host__ import host_coroutines
import json
TRACE = []
def op(name):
    result = json.loads((yield exchange(json.dumps({'phase': name, 'trace': TRACE}))))
    TRACE.append([name, result])
    return result
class Scope:
    async def __aenter__(self):
        return self
    async def __aexit__(self, typ, error, tb):
        await op('context-close')
        return False
"""


def cancel(reason, *results):
    """Explicit transport input; presence of empty reason is significant."""
    return {'native_cancel': reason, 'native_results': list(results)}


def reply(value, request_id=None):
    """Explicit future reply; an optional ID allows literal stale delivery probes."""
    result = {'reply': value}
    if request_id is not None:
        result['id'] = request_id
    return result


# These expectations are authored independently of candidate runtime output.
CASES = {
    'caught-empty-repeat-stale': (
        """async def body():
    try:
        await op('body')
    except ValueError as error:
        TRACE.append(['caught', str(error)])
        return 'caught'
    finally:
        await op('finally')
""",
        [cancel(''), cancel('REPEAT', reply('late', 0), reply('clean', 1))],
        ['body', 'finally'],
        '',
        {'value': 'caught', 'trace': [['caught', ''], ['finally', 'clean']]},
        None,
    ),
    'cleanup-first-no-reply': (
        """async def body():
    try:
        return 'done'
    finally:
        await op('finally')
""",
        [cancel(''), cancel('REPEAT'), 'clean'],
        ['finally', 'finally', 'finally'],
        '',
        None,
        'ValueError: ',
    ),
    'repeat-no-reply': (
        """async def body():
    try:
        await op('body')
    except ValueError:
        return 'caught'
    finally:
        await op('finally')
""",
        [cancel('FIRST'), cancel('REPEAT'), 'clean'],
        ['body', 'finally', 'finally'],
        'FIRST',
        {'value': 'caught', 'trace': [['finally', 'clean']]},
        None,
    ),
    'same-batch-cancel-wins': (
        """async def body():
    try:
        await op('body')
    except ValueError as error:
        return str(error)
""",
        [cancel('HOST', reply({'raise': 'IO'}))],
        ['body'],
        'HOST',
        {'value': 'HOST', 'trace': []},
        None,
    ),
    'uncaught': (
        """async def body():
    try:
        await op('body')
    finally:
        await op('finally')
""",
        [cancel('FIRST'), 'clean'],
        ['body', 'finally'],
        'FIRST',
        None,
        'ValueError: FIRST',
    ),
    'cleanup-first': (
        """async def body():
    try:
        return 'done'
    finally:
        await op('finally')
""",
        [cancel('FIRST', reply('clean'))],
        ['finally'],
        'FIRST',
        None,
        'ValueError: FIRST',
    ),
    'context-first-nested': (
        """async def body():
    try:
        async with Scope():
            return 'done'
    finally:
        await op('outer-finally')
""",
        [cancel('FIRST', reply('inner')), cancel('SECOND', reply('outer'))],
        ['context-close', 'outer-finally'],
        'FIRST',
        None,
        'ValueError: FIRST',
    ),
    'cleanup-error': (
        """async def body():
    try:
        await op('body')
    finally:
        try:
            await op('inner')
        finally:
            await op('outer')
""",
        [cancel('FIRST'), {'raise': 'CLEANUP'}, cancel('REPEAT', reply('outer'))],
        ['body', 'inner', 'outer'],
        'FIRST',
        None,
        'ValueError: CLEANUP',
    ),
    'error-before-cleanup-cancel': (
        """async def body():
    try:
        await op('body')
    finally:
        await op('finally')
""",
        [{'raise': 'BODY'}, cancel('HOST', reply('clean'))],
        ['body', 'finally'],
        'HOST',
        None,
        'ValueError: BODY',
    ),
    'error-after-caught-cancel': (
        """async def body():
    try:
        await op('body')
    except ValueError:
        await op('recovery')
""",
        [cancel('FIRST'), cancel('REPEAT', reply({'raise': 'RECOVERY'}))],
        ['body', 'recovery'],
        'FIRST',
        None,
        'ValueError: RECOVERY',
    ),
    'cleanup-exit-before-later-handler': (
        """async def body():
    try:
        pass
    finally:
        await op('finally')
    try:
        await op('body')
    except ValueError as error:
        return str(error)
""",
        [cancel('FIRST', reply('clean'))],
        ['finally'],
        'FIRST',
        None,
        'ValueError: FIRST',
    ),
}


def main():
    """Every wait is dumped, disposed, and independently resumed twice in restored mode."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--backend', choices=('native', 'wasm', 'both'), default='both')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm') if args.backend == 'both' else (args.backend,):
        for mode in ('uninterrupted', 'restored'):
            for name, (body, batches, phases, reason, value, error) in CASES.items():
                key = f'{backend}-{mode}-{name}'
                result = execute(
                    PRELUDE + body + "\nvalue = body()._native1()\njson.dumps({'value':value,'trace':TRACE})\n",
                    batches,
                    backend,
                    mode,
                    args.output / key,
                )
                results[key] = result
                (args.output / 'partial.json').write_text(json.dumps(results, indent=2) + '\n')
                assert [v['phase'] for v in result['views'][:-1]] == phases, result
                terminal = result['views'][-1]
                assert terminal['status'] == 'cancelled' and terminal['reason'] == reason, result
                if error is None:
                    assert terminal['error'] is None and json.loads(terminal['value']) == value, result
                else:
                    assert terminal['value'] is None and terminal['error'].splitlines()[-1] == error, result
    (args.output / 'report.json').write_text(json.dumps({'status': 'PASS', 'results': results}, indent=2) + '\n')
    print(f'PASS {len(results)} literal native cancellation executions')


if __name__ == '__main__':
    main()
