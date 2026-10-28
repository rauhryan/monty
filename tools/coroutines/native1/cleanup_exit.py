"""Literal cleanup-exit cancellation regressions; no candidate-derived oracle."""

import argparse
import json
from pathlib import Path

from adapter import execute
from cancellation import PRELUDE, cancel, reply

ENTRY = """
async def entry():
    try:
        value = await body()
        TRACE.append('FORBIDDEN-entry-return')
        return value
    except ValueError as error:
        TRACE.append(['delivered', str(error)])
        return 'caught'
value = entry()._native1()
json.dumps({'value':value,'trace':TRACE})
"""

# Expected traces are literal policy assertions, authored before executing the fix.
CASES = {
    'normal-no-body-effect': (
        """async def body():
    try:
        pass
    finally:
        await op('cleanup')
    TRACE.append('FORBIDDEN-before-delivery')
    await op('purchase')
""",
        [cancel('', reply('ok'))],
        ['cleanup'],
        [['cleanup', 'ok'], ['delivered', '']],
    ),
    'terminal-return-exited-inner-handler': (
        """async def body():
    try:
        try:
            return ['pending-result']
        finally:
            await op('cleanup')
    except ValueError:
        TRACE.append('FORBIDDEN-exited-handler')
""",
        [cancel('FIRST'), cancel('REPEAT'), 'ok'],
        ['cleanup', 'cleanup', 'cleanup'],
        [['cleanup', 'ok'], ['delivered', 'FIRST']],
    ),
    'nested-returns-and-outer-cleanup': (
        """async def helper():
    try:
        return 'helper'
    finally:
        await op('inner')
async def body():
    try:
        return 'pending'
    finally:
        TRACE.append(await helper())
        await op('outer')
        TRACE.append('outer-done')
""",
        [cancel('FIRST', reply('i')), cancel('REPEAT', reply('o'))],
        ['inner', 'outer'],
        [['inner', 'i'], 'helper', ['outer', 'o'], 'outer-done', ['delivered', 'FIRST']],
    ),
    'nested-inline-return': (
        """async def body():
    try:
        try:
            return 'pending'
        finally:
            await op('inner')
    finally:
        await op('outer')
""",
        [cancel('FIRST', reply('i')), cancel('REPEAT', reply('o'))],
        ['inner', 'outer'],
        [['inner', 'i'], ['outer', 'o'], ['delivered', 'FIRST']],
    ),
    'return-in-finally': (
        """async def body():
    try:
        raise TypeError('discarded-by-authored-return')
    finally:
        await op('cleanup')
        return ['replacement']
""",
        [cancel('FIRST', reply('ok'))],
        ['cleanup'],
        [['cleanup', 'ok'], ['delivered', 'FIRST']],
    ),
    'normal-applicable-handler': (
        """async def body():
    try:
        try:
            pass
        finally:
            await op('cleanup')
        TRACE.append('FORBIDDEN-before-catch')
    except ValueError as error:
        TRACE.append(['body-caught', str(error)])
        await op('authored-after-delivery')
    raise ValueError('outer')
""",
        [cancel('FIRST', reply('ok')), cancel('REPEAT', reply('after'))],
        ['cleanup', 'authored-after-delivery'],
        [['cleanup', 'ok'], ['body-caught', 'FIRST'], ['authored-after-delivery', 'after'], ['delivered', 'outer']],
    ),
    'context-normal': (
        """async def body():
    async with Scope():
        pass
    TRACE.append('FORBIDDEN-context')
    await op('purchase')
""",
        [cancel('FIRST', reply('ok'))],
        ['context-close'],
        [['context-close', 'ok'], ['delivered', 'FIRST']],
    ),
    'context-suppression': (
        """class Suppress:
    async def __aenter__(self):
        return self
    async def __aexit__(self, typ, error, tb):
        await op('context-close')
        return True
async def body():
    async with Suppress():
        raise TypeError('suppressed')
    TRACE.append('FORBIDDEN-suppression')
""",
        [cancel('FIRST', reply('ok'))],
        ['context-close'],
        [['context-close', 'ok'], ['delivered', 'FIRST']],
    ),
    'nested-context-return': (
        """async def body():
    async with Scope():
        async with Scope():
            return 'pending'
""",
        [cancel('FIRST', reply('i')), cancel('REPEAT', reply('o'))],
        ['context-close', 'context-close'],
        [['context-close', 'i'], ['context-close', 'o'], ['delivered', 'FIRST']],
    ),
    'yield-from-helper': (
        """def helper():
    result = yield from op('cleanup')
    TRACE.append(['helper-result', result])
    return result
async def body():
    try:
        pass
    finally:
        TRACE.append(['cleanup-result', await helper()])
    TRACE.append('FORBIDDEN-delegation')
""",
        [cancel('FIRST', reply('ok'))],
        ['cleanup'],
        [['cleanup', 'ok'], ['helper-result', 'ok'], ['cleanup-result', 'ok'], ['delivered', 'FIRST']],
    ),
    'matched-original-error-boundary': (
        """async def body():
    try:
        try:
            raise TypeError('handled-original')
        finally:
            await op('cleanup')
    except TypeError as error:
        TRACE.append('FORBIDDEN-handled-error-body')
""",
        [cancel('FIRST', reply('ok'))],
        ['cleanup'],
        [['cleanup', 'ok'], ['delivered', 'FIRST']],
    ),
}
for transfer in ('break', 'continue'):
    for placement in ('body', 'finally'):
        source = (
            'async def body():\n    for i in range(2):\n        try:\n'
            + (f'            {transfer}\n' if placement == 'body' else '            pass\n')
            + "        finally:\n            await op('cleanup')\n"
            + (f'            {transfer}\n' if placement == 'finally' else '')
            + "    TRACE.append('FORBIDDEN-loop-exit')\n"
        )
        CASES[f'{transfer}-from-{placement}'] = (
            source,
            [cancel('FIRST', reply('ok'))],
            ['cleanup'],
            [['cleanup', 'ok'], ['delivered', 'FIRST']],
        )

CASES['implicit-terminal'] = (
    """async def body():
    try:
        pass
    finally:
        await op('cleanup')
""",
    [cancel('FIRST', reply('ok'))],
    ['cleanup'],
    [['cleanup', 'ok'], ['delivered', 'FIRST']],
)
for transfer in ('break', 'continue'):
    CASES[f'while-{transfer}-from-finally'] = (
        "async def body():\n    while True:\n        try:\n            pass\n        finally:\n            await op('cleanup')\n"
        + f'            {transfer}\n'
        + "    TRACE.append('FORBIDDEN-while-exit')\n",
        [cancel('FIRST', reply('ok'))],
        ['cleanup'],
        [['cleanup', 'ok'], ['delivered', 'FIRST']],
    )
CASES['matched-error-reraise-policy'] = (
    """async def body():
    try:
        try:
            raise TypeError('matched-original')
        finally:
            await op('cleanup')
    except TypeError:
        raise
""",
    [cancel('FIRST', reply('ok'))],
    ['cleanup'],
    [['cleanup', 'ok'], ['delivered', 'FIRST']],
)

ERROR_CASES = {
    'escaping-original-error': (
        """async def body():
    try:
        raise TypeError('ORIGINAL')
    finally:
        await op('cleanup')
""",
        [cancel('FIRST', reply('ok'))],
        ['cleanup'],
        'TypeError: ORIGINAL',
    ),
    'escaping-cleanup-error-outer-completes': (
        """async def body():
    try:
        try:
            return ['pending']
        finally:
            await op('inner')
            raise TypeError('CLEANUP')
    finally:
        await op('outer')
""",
        [cancel('FIRST', reply('i')), cancel('REPEAT', reply('o'))],
        ['inner', 'outer'],
        'TypeError: CLEANUP',
    ),
    'context-unsuppressed-original': (
        """async def body():
    async with Scope():
        raise TypeError('ORIGINAL')
""",
        [cancel('FIRST', reply('ok'))],
        ['context-close'],
        'TypeError: ORIGINAL',
    ),
}

# Repair5: matching is not handling. Both prospective handlers are bypassed
# before expression evaluation; retain their original diagnostic instead.
for name, error in (
    ('matched-original-error-boundary', 'TypeError: handled-original'),
    ('matched-error-reraise-policy', 'TypeError: matched-original'),
):
    source, batches, phases, _ = CASES.pop(name)
    ERROR_CASES[name] = (source, batches, phases, error)


def main():
    """Every restored wait is independently disposed and resumed twice."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--backend', choices=('native', 'wasm', 'both'), default='both')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm') if args.backend == 'both' else (args.backend,):
        for mode in ('uninterrupted', 'restored'):
            for name, (source, batches, phases, trace) in CASES.items():
                key = f'{backend}-{mode}-{name}'
                result = execute(PRELUDE + source + ENTRY, batches, backend, mode, args.output / key)
                results[key] = result
                (args.output / 'partial.json').write_text(json.dumps(results, indent=2) + '\n')
                assert [v['phase'] for v in result['views'][:-1]] == phases, result
                assert all(v['phase'] != 'purchase' for v in result['views'][:-1]), result
                terminal = result['views'][-1]
                assert terminal['status'] == 'cancelled' and terminal['reason'] == batches[0]['native_cancel'], result
                assert terminal['error'] is None, result
                assert json.loads(terminal['value']) == {'value': 'caught', 'trace': trace}, result
                assert 'FORBIDDEN' not in terminal['value'], result
    for backend in ('native', 'wasm') if args.backend == 'both' else (args.backend,):
        for mode in ('uninterrupted', 'restored'):
            for name, (source, batches, phases, error) in ERROR_CASES.items():
                key = f'{backend}-{mode}-{name}'
                result = execute(PRELUDE + source + ENTRY, batches, backend, mode, args.output / key)
                results[key] = result
                assert [v['phase'] for v in result['views'][:-1]] == phases, result
                terminal = result['views'][-1]
                assert terminal['status'] == 'cancelled' and terminal['reason'] == 'FIRST', result
                assert terminal['value'] is None and terminal['error'].splitlines()[-1] == error, result
    (args.output / 'report.json').write_text(json.dumps({'status': 'PASS', 'results': results}, indent=2) + '\n')
    print(f'PASS {len(results)} cleanup-exit executions')


if __name__ == '__main__':
    main()
