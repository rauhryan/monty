"""Literal error-first dispatch policy tests for native and actual Wasm."""

import argparse
import json
from pathlib import Path

from adapter import execute
from cancellation import PRELUDE, cancel, reply
from cleanup_exit import ENTRY

# Matching expressions must not run, even if their result would not match.
HELPERS = {
    'literal': ('', 'ValueError'),
    'explicit': ("def choose():\n    TRACE.append('FORBIDDEN')\n    return ValueError\n", 'choose()'),
    'implicit': ("def choose():\n    TRACE.append('FORBIDDEN')\n", 'choose()'),
    'sync-match': ("def choose():\n    TRACE.append('FORBIDDEN')\n    return TypeError\n", 'choose()'),
    'async-match': ("async def choose():\n    await op('purchase')\n    return TypeError\n", 'await choose()'),
    'async-unmatched': ("async def choose():\n    await op('purchase')\n    return ValueError\n", 'await choose()'),
    'rethrow': ('', 'TypeError'),
}


def error_source(helper, expression, cleanup_error=False):
    """Assert no matching/body work in enclosing cleanup before terminal error."""
    trigger = "raise TypeError('ORIGINAL')" if not cleanup_error else 'pass'
    cleanup = "\n                raise TypeError('CLEANUP')" if cleanup_error else ''
    return (
        helper
        + f"""async def body():
    try:
        try:
            try:
                {trigger}
            finally:
                await op('cleanup'){cleanup}
        except {expression} as error:
            TRACE.append('FORBIDDEN-handler')
            raise
    finally:
        assert TRACE == [['cleanup', 'ok']]
        await op('outer')
        assert TRACE == [['cleanup', 'ok'], ['outer', 'done']]
"""
    )


def main():
    """Run literal policy expectations, including absence of any extra host turn."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm'):
        for mode in ('uninterrupted', 'restored'):
            for name, (helper, expression) in HELPERS.items():
                for cleanup_error in (False, True):
                    key = f'{backend}-{mode}-{name}-{cleanup_error}'
                    result = execute(
                        PRELUDE + error_source(helper, expression, cleanup_error) + ENTRY,
                        [cancel('', reply('ok')), cancel('REPEAT', reply('done'))],
                        backend,
                        mode,
                        args.output / key,
                    )
                    assert [v['phase'] for v in result['views'][:-1]] == ['cleanup', 'outer']
                    terminal = result['views'][-1]
                    expected = 'TypeError: CLEANUP' if cleanup_error else 'TypeError: ORIGINAL'
                    assert terminal['status'] == 'cancelled' and terminal['reason'] == ''
                    assert terminal['value'] is None and terminal['error'].splitlines()[-1] == expected
                    results[key] = result
                    (args.output / 'report.json').write_text(
                        json.dumps({'status': 'PASS error-first policy', 'results': results}, indent=2) + '\n'
                    )
    print(f'PASS {len(results)} dispatch executions')


if __name__ == '__main__':
    main()
