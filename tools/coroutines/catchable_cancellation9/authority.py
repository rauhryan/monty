"""Real capability denial during prospective selection for the delivered signal."""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'tools/coroutines/capabilities'), str(ROOT / 'tools/coroutines/capabilities/tests')]
from demo import provision  # noqa: E402
from host import Finished  # noqa: E402
from journal import Denied  # noqa: E402
from repair import record, start  # noqa: E402

BODY = """
TRACE = []
async def choose():
    TRACE.append('selector-for-signal')
    await charge(1)
    return ValueError
async def body():
    try:
        try:
            raise TypeError('ORIGINAL')
        finally:
            await release()
    except await choose():
        TRACE.append('FORBIDDEN')
async def entry():
    try:
        await body()
    except ValueError as error:
        TRACE.append(['caught', str(error)])
    return TRACE
"""
EXPECTED = ['selector-for-signal', ['caught', 'operation unavailable']]
OUT = (
    Path(sys.argv[1])
    if len(sys.argv) > 1
    else ROOT / 'playground/coroutine-implementation/catchable-cancellation9/authority'
)
OUT.mkdir(exist_ok=False)
(OUT / 'frozen-input.json').write_text(
    json.dumps({'body': BODY, 'cancel': 'FIRST', 'repeat': 'REPEAT', 'expected': EXPECTED}, indent=2) + '\n'
)
results = {}
for backend in ('native', 'wasm'):
    directory = OUT / backend
    directory.mkdir()
    host, handles = provision(directory)
    try:
        cleanup = start(directory, host, handles, backend, BODY)
        first = host.advance(cleanup, cancel='FIRST')
        repeat = host.advance(first, cancel='REPEAT')
        purchase = host.advance(repeat)
        assert host.state(purchase).request['op'] == 'charge'
        try:
            host.settle(purchase)
        except Denied as error:
            assert str(error) == 'operation unavailable'
        else:
            raise AssertionError('postdelivery selector acquired host authority')
        terminal = host.advance(purchase)
        assert isinstance(terminal, Finished)
        assert terminal.outcome['status'] == 'cancelled'
        assert terminal.outcome['reason'] == 'FIRST'
        assert terminal.outcome['error'] is None
        assert terminal.outcome['interrupted_error'] == {
            'type': 'TypeError',
            'message': 'ORIGINAL',
            'message_truncated': False,
            'representation': 'metadata-only',
        }
        assert json.loads(terminal.outcome['value']) == EXPECTED
        assert host.service.count('charge') == 0 and host.service.count('release') == 1
        results[backend] = record(host, terminal)
    finally:
        host.journal.close()
        host.service.close()
(OUT / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
print('PASS two actual postdelivery selector authority denials; host diagnostic is operation unavailable')
