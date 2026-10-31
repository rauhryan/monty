"""Resume authenticated, unmodified historical snapshots; never fabricate version headers."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
E = ROOT / 'playground/coroutine-implementation/catchable-cancellation9'
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else E / 'legacy'
OUT.mkdir(exist_ok=False)
auth = json.loads((E / 'historical-old-snapshot-authentication.json').read_text())
selected = {}
for row in auth:
    backend = 'wasm' if '/wasm-' in row['snapshot'] else 'native'
    state = 'ordinary' if row['snapshot'].endswith('cut-0.bin') else 'pending'
    selected[(row['version'], backend, state)] = row
report_path = ROOT / 'playground/coroutine-implementation/cleanup-exit6/final/cancellation/report.json'
report = json.loads(report_path.read_text())['results']
for backend in ('native', 'wasm'):
    for state, name, cut in [
        ('ordinary', 'caught-empty-repeat-stale', 0),
        ('delivered', 'caught-empty-repeat-stale', 1),
        ('pending', 'cleanup-first-no-reply', 1),
    ]:
        case = report[f'{backend}-restored-{name}']
        row = next(x for x in case['evidence'] if x.get('snapshot', '').endswith(f'/cut-{cut}.bin'))
        selected[(22, backend, state)] = row | {
            'version': 22,
            'producer_report': str(report_path),
            'producer_report_sha256': hashlib.sha256(report_path.read_bytes()).hexdigest(),
        }
receipts = []
for (version, backend, state), row in sorted(selected.items()):
    snapshot = Path(row['snapshot'])
    assert hashlib.sha256(snapshot.read_bytes()).hexdigest() == row['sha256']
    command = (
        [str(ROOT / 'playground/coroutine-baseline/target/debug/native1')]
        if backend == 'native'
        else ['node', str(ROOT / 'tools/coroutines/native1/wasm.mjs')]
    )
    for action in ['ordinary', 'cancel'] if state == 'ordinary' else ['refuse']:
        batch = (
            '"ok"'
            if action != 'cancel'
            else json.dumps({'native_cancel': 'FIRST', 'native_results': [{'reply': 'MUST-NOT-APPLY'}]})
        )
        output = OUT / f'{version}-{backend}-{state}-{action}.bin'
        result = subprocess.run(
            command + ['resume', str(snapshot), batch, str(output)], capture_output=True, text=True, timeout=60
        )
        passed = (
            result.returncode == 0
            if action == 'ordinary'
            else result.returncode != 0
            and 'native cancellation requires catchable cancellation policy 7' in result.stderr
            and not result.stdout
        )
        receipts.append(
            {
                'version': version,
                'backend': backend,
                'state': state,
                'action': action,
                'provenance': row,
                'argv': result.args,
                'exit': result.returncode,
                'stdout': result.stdout,
                'stderr': result.stderr,
                'passed': passed,
            }
        )
        if output.exists():
            dispose = subprocess.run(command + ['dispose', str(output)], capture_output=True, text=True, timeout=60)
            receipts[-1]['dispose'] = {
                'argv': dispose.args,
                'exit': dispose.returncode,
                'stdout': dispose.stdout,
                'stderr': dispose.stderr,
            }
            assert dispose.returncode == 0
(OUT / 'report.json').write_text(json.dumps(receipts, indent=2) + '\n')
print(json.dumps({'passed': sum(r['passed'] for r in receipts), 'total': len(receipts)}))
assert all(r['passed'] for r in receipts)
