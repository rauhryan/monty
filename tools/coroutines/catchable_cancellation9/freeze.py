"""Materialize authored supplemental sources and all-cut coordinates before execution."""

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'playground/coroutine-implementation/catchable-cancellation9/inputs'
core = json.loads((ROOT / 'tools/coroutines/catchable_cancellation7/cases.json').read_text())['cases']
prelude = core[0]['source'].split('async def body():')[0]
entry = core[0]['source'].split('\nasync def entry():')[1]
extras = []


def add(name, body, phases, trace, diagnostic, batches=None):
    """Literal observations are specified independently of candidate execution."""
    extras.append(
        {
            'id': name,
            'source': prelude + body + '\nasync def entry():' + entry,
            'batches': batches or [{'native_cancel': 'FIRST', 'native_results': [{'reply': 'ok'}]}],
            'expected': {
                'effects': [{'phase': p, 'trace': t} for p, t in phases],
                'terminal': {
                    'status': 'cancelled',
                    'reason': 'FIRST',
                    'value': {'value': 'caught', 'trace': trace},
                    'error': None,
                    'interrupted_error': diagnostic,
                    'value_export_error': None,
                },
            },
        }
    )


def diag(message, truncated=False):
    return {'type': 'TypeError', 'message': message, 'message_truncated': truncated, 'representation': 'metadata-only'}


for name, setup, expr, message, truncated in [
    ('null', '', 'TypeError()', None, False),
    ('empty', '', "TypeError('')", '', False),
    ('cap4096', '', "TypeError('a' * 4096)", 'a' * 4096, False),
    ('cap4097', '', "TypeError('a' * 4097)", 'a' * 4096, True),
    ('multibyte', '', "TypeError('a' * 4095 + 'é')", 'a' * 4095, True),
    ('multibyte-exact', '', "TypeError('é' * 2048)", 'é' * 2048, False),
    ('args-alias', '    token = []\n', 'TypeError([token, token])', None, False),
    ('args-cycle', '    a = []\n    a.append(a)\n', 'TypeError(a)', None, False),
    ('args-generator', '', "TypeError(op('never'))", None, False),
]:
    add(
        'metadata-' + name,
        f"async def body():\n{setup}    try:\n        raise {expr}\n    finally:\n        await op('cleanup')\n",
        [('cleanup', [])],
        [['cleanup', 'ok'], ['caught', 'FIRST']],
        diag(message, truncated),
    )
add(
    'current-over-S',
    """async def choose():
    try:
        raise TypeError('CURRENT')
    finally:
        await op('cleanup')
async def body():
    try:
        raise TypeError('ORIGINAL')
    except await choose():
        TRACE.append('FORBIDDEN')
""",
    [('cleanup', [])],
    [['cleanup', 'ok'], ['caught', 'FIRST']],
    diag('CURRENT'),
)
add(
    'handled-outer',
    """async def body():
    try:
        raise TypeError('HANDLED')
    except TypeError:
        await op('body')
""",
    [('body', [])],
    [['caught', 'FIRST']],
    None,
    [{'native_cancel': 'FIRST', 'native_results': []}],
)
# Helper catches locally, then ordinary original matching continues.
add(
    'helper-local',
    """async def choose():
    try:
        await op('selector')
    except ValueError as error:
        TRACE.append(['local', str(error)])
    return TypeError
async def body():
    try:
        raise TypeError('ORIGINAL')
    except await choose() as error:
        TRACE.append(['original', str(error)])
        return 'recovered'
""",
    [('selector', [])],
    [['local', 'FIRST'], ['original', 'ORIGINAL']],
    diag('ORIGINAL'),
    [{'native_cancel': 'FIRST', 'native_results': [{'reply': 'ignored'}]}],
)
extras[-1]['expected']['terminal']['value']['value'] = 'recovered'
OUT.mkdir(exist_ok=False)
manifest = []
for case in core + extras:
    directory = OUT / case['id']
    directory.mkdir()
    source = case['source'].encode()
    batches = (json.dumps(case['batches'], ensure_ascii=False, indent=2) + '\n').encode()
    (directory / 'guest.py').write_bytes(source)
    (directory / 'batches.json').write_bytes(batches)
    for backend in ('native', 'wasm'):
        for cut in range(len(case['expected']['effects'])):
            manifest.append(
                {
                    'case': case['id'],
                    'backend': backend,
                    'cut': cut,
                    'source_sha256': hashlib.sha256(source).hexdigest(),
                    'batches_sha256': hashlib.sha256(batches).hexdigest(),
                }
            )
(OUT / 'supplemental.json').write_text(json.dumps(extras, ensure_ascii=False, indent=2) + '\n')
(OUT / 'all-cuts.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(f'Frozen {len(core)} core and {len(extras)} supplemental sources; {len(manifest)} cuts')
