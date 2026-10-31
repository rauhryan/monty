"""Run frozen repair10 semantics/faults on actual instrumented Wasm, retaining every receipt."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(sys.argv[1])
OUT.mkdir(parents=True, exist_ok=False)
SOURCE = ROOT / 'tools/coroutines/catchable_cancellation10/ownership.py'
MODES = [
    'uncaught',
    'later',
    'depth',
    'terminal',
    'delivery-internal',
    'delivery-uncatchable',
    'projection-internal',
    'projection-uncatchable',
]


def invoke(action, mode, source, output, cut=0):
    """Retain failures as well as successes, with actual process identities."""
    argv = [
        'node',
        str(ROOT / 'tools/coroutines/catchable_cancellation10/wasm.mjs'),
        action,
        mode,
        str(source),
        str(output),
        str(cut),
    ]
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    stdout, stderr = process.communicate(timeout=60)
    receipt = {'pid': process.pid, 'argv': argv, 'exit': process.returncode, 'stdout': stdout, 'stderr': stderr}
    path = OUT / f'{mode}-{len(list(OUT.glob("*.receipt.json")))}.receipt.json'
    path.write_text(json.dumps(receipt, indent=2) + '\n')
    assert process.returncode == 0, receipt
    return json.loads(stdout), str(path), stderr


def decode(arena, index):
    """Decode just the scalar/dict fields needed by the literal assertions."""
    node = arena['nodes'][index]
    if node['tag'] == 'none':
        return None
    if node['tag'] in ('text', 'boolean'):
        return node['val']
    assert node['tag'] == 'dict', node
    return {decode(arena, pair['key']): decode(arena, pair['value']) for pair in node['val']}


def check(mode, result, stderr):
    """Expectations authored before any candidate run, not derived from observations."""
    event = result['event']
    if mode in ('uncaught', 'later', 'depth'):
        assert event['tag'] == 'complete', event
        value = decode(event['val']['values'], event['val']['value'])
        assert value['status'] == 'cancelled' and value['reason'] == 'FIRST'
        assert value['value'] is None
        assert value['interrupted_error'] == {
            'type': 'TypeError',
            'message': None,
            'message_truncated': False,
            'representation': 'metadata-only',
        }
        assert value['value_export_error'] == ('export-depth-limit' if mode == 'depth' else None)
        if mode == 'depth':
            assert value['error'] is None
        else:
            assert value['error'].strip().splitlines()[-1] == (
                'ValueError: FIRST' if mode == 'uncaught' else 'TypeError: LATER'
            )
    else:
        assert event['tag'] == 'error', event
        marker = (
            'REPAIR10_INTERNAL'
            if 'internal' in mode
            else 'REPAIR10_UNCATCHABLE'
            if 'uncatchable' in mode
            else 'REPAIR10_EXPORT_TERMINAL'
        )
        assert marker in json.dumps(event) and 'ValueError: FIRST' not in json.dumps(event)
    if mode in ('depth', 'terminal'):
        assert 'repair10-export:' in stderr and 'dropped: true' in stderr, stderr
        pins = 2 if mode == 'depth' else 1
        assert f'peak_pins: {pins}, retired_pins: {pins}' in stderr, stderr
    elif mode.startswith(('delivery-', 'projection-')):
        assert f'repair10-fault: {mode}' in stderr, stderr


rows = []
for mode in MODES:
    result, receipt, stderr = invoke('run', mode, SOURCE, OUT / 'unused')
    check(mode, result, stderr)
    phases = ['cleanup'] if mode == 'uncaught' or mode.startswith('delivery-') else ['cleanup', 'recovery']
    assert result['phases'] == phases
    snapshot = OUT / f'{mode}-cut0.bin'
    current, producer, _ = invoke('start', mode, SOURCE, snapshot)
    for cut, phase in enumerate(phases):
        assert current['event']['phase'] == phase
        before = snapshot.read_bytes()
        disposed, disposal, _ = invoke('dispose', mode, snapshot, OUT / 'unused')
        assert disposed['disposed'] and disposed['event']['tag'] == 'resolve-futures'
        branches = []
        for branch in range(2):
            target = OUT / f'{mode}-cut{cut}-branch{branch}.bin'
            step, resumed, stderr = invoke('resume', mode, snapshot, target, cut)
            receipts = [resumed]
            if cut + 1 < len(phases):
                assert step['event']['phase'] == phases[cut + 1]
                _, d, _ = invoke('dispose', mode, target, OUT / 'unused')
                receipts.append(d)
                step, resumed, stderr = invoke('resume', mode, target, OUT / 'unused', cut + 1)
                receipts.append(resumed)
            check(mode, step, stderr)
            branches.append(receipts)
        rows.append(
            {
                'mode': mode,
                'cut': cut,
                'phase': phase,
                'sha256': hashlib.sha256(before).hexdigest(),
                'snapshot': str(snapshot),
                'producer': producer,
                'disposal': disposal,
                'branches': branches,
            }
        )
        assert snapshot.read_bytes() == before
        if cut + 1 < len(phases):
            target = OUT / f'{mode}-cut{cut + 1}.bin'
            current, producer, _ = invoke('resume', mode, snapshot, target, cut)
            snapshot = target
(OUT / 'report.json').write_text(json.dumps({'status': 'PASS', 'modes': MODES, 'cuts': rows}, indent=2) + '\n')
print(f'PASS {len(MODES)} actual-Wasm modes, {len(rows)} cuts, two independent suffixes each')
