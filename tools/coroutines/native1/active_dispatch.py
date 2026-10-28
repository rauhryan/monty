"""Literal regressions for cancellation during already-entered exception matching."""

import argparse
import json
from pathlib import Path

from adapter import execute
from cancellation import PRELUDE, cancel, reply
from cleanup_exit import ENTRY


def cases():
    """State independent expected errors and traces before executing the candidate."""
    for implicit in (False, True):
        for match in (False, True):
            for outer in (False, True):
                typ = 'TypeError' if match else 'ValueError'
                statement = 'pass' if implicit else 'return ' + typ
                expr = '(await choose()) or ' + typ if implicit else 'await choose()'
                helper = (
                    f"""async def choose():
    try:
        {statement}
    finally:
        await op('cleanup')
    TRACE.append('FORBIDDEN-selector')
    await op('purchase')
"""
                    if not implicit
                    else """async def choose():
    try:
        pass
    finally:
        await op('cleanup')
"""
                )
                body = f"""async def body():
    original = TypeError('ORIGINAL')
    alias = original
    try:
        raise original
    except {expr}:
        TRACE.append('FORBIDDEN-handler')
"""
                if outer:
                    body = body.replace(
                        '    try:\n        raise original', '    try:\n        try:\n            raise original'
                    ).replace(
                        '    except ' + expr + ':\n        TRACE', '        except ' + expr + ':\n            TRACE'
                    )
                    body += "    finally:\n        assert alias is original\n        assert TRACE == [['cleanup', 'ok']]\n        await op('outer')\n"
                batches = [cancel('', reply('ok'))] + ([cancel('REPEAT', reply('done'))] if outer else [])
                yield f'selector-{implicit}-{match}-{outer}', helper + body, batches, 'TypeError: ORIGINAL', None
    # An implicit selector must not resume the rest of a valid match expression.
    source = """async def choose():
    try:
        pass
    finally:
        await op('cleanup')
async def effect():
    await op('purchase')
    return ValueError
async def body():
    try:
        raise TypeError('ORIGINAL')
    except (await choose()) or (await effect()):
        TRACE.append('FORBIDDEN-handler')
"""
    yield 'expression-continuation', source, [cancel('', reply('ok'))], 'TypeError: ORIGINAL', None
    source = """async def choose():
    try:
        pass
    finally:
        await op('cleanup')
        return ValueError
async def body():
    try:
        raise TypeError('ORIGINAL')
    except await choose():
        TRACE.append('FORBIDDEN-handler')
"""
    yield 'selector-finally-return', source, [cancel('', reply('ok'))], 'TypeError: ORIGINAL', None
    for implicit in (False, True):
        statement = 'pass' if implicit else 'return ValueError'
        expression = '(await choose()) or ValueError' if implicit else 'await choose()'
        source = f"""async def choose():
    try:
        {statement}
    finally:
        await op('cleanup')
async def body():
    try:
        pass
    finally:
        original = TypeError('ORIGINAL')
        try:
            try:
                raise original
            except {expression}:
                TRACE.append('FORBIDDEN-handler')
        except TypeError as active:
            assert active is original
            TRACE.append('protected-original')
        await op('outer')
    TRACE.append('FORBIDDEN-after')
"""
        yield (
            f'protected-{implicit}',
            source,
            [cancel('', reply('ok')), cancel('REPEAT', reply('done'))],
            None,
            [['cleanup', 'ok'], 'protected-original', ['outer', 'done'], ['delivered', '']],
        )
    for outcome in ('False', 'True', "raise TypeError('EXIT')"):
        stmt = outcome if outcome.startswith('raise') else 'return ' + outcome
        source = f"""class Scope:
    def __enter__(self):
        return self
    def __exit__(self, typ, error, tb):
        assert str(error) == 'ORIGINAL'
        TRACE.append('exit')
        {stmt}
async def body():
    with Scope():
        try:
            raise TypeError('ORIGINAL')
        finally:
            await op('cleanup')
    TRACE.append('FORBIDDEN-after')
    await op('purchase')
"""
        error = (
            None if outcome == 'True' else ('TypeError: EXIT' if outcome.startswith('raise') else 'TypeError: ORIGINAL')
        )
        trace = [['cleanup', 'ok'], 'exit', ['delivered', '']] if outcome == 'True' else None
        yield 'sync-' + outcome.split()[0], source, [cancel('', reply('ok'))], error, trace


def main():
    """Exercise both transports and independent same-cut restore branches."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm'):
        for mode in ('uninterrupted', 'restored'):
            for name, source, batches, error, trace in cases():
                key = f'{backend}-{mode}-{name}'
                result = execute(PRELUDE + source + ENTRY, batches, backend, mode, args.output / key)
                results[key] = result
                (args.output / 'report.json').write_text(json.dumps(results, indent=2))
                terminal = result['views'][-1]
                assert terminal['status'] == 'cancelled' and terminal['reason'] == '', terminal
                if error:
                    assert terminal['value'] is None and terminal['error'].splitlines()[-1] == error, terminal
                else:
                    assert terminal['error'] is None and json.loads(terminal['value']) == {
                        'value': 'caught',
                        'trace': trace,
                    }, terminal
    print(f'PASS {len(results)} active dispatch executions')


if __name__ == '__main__':
    main()
