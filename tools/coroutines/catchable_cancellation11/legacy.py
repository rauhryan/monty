"""Authenticate genuine delivered v19/20/21 snapshots, then check explicit refusal literals."""

import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'playground/coroutine-implementation/catchable-cancellation-compatibility11'
REFUSAL = 'native cancellation requires catchable cancellation policy 7'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    selected = []
    for version, folder in [
        (19, 'cancellation2/cancellation-final'),
        (20, 'cleanup-exit4/final/cancellation'),
        (21, 'cleanup-exit5/final/cancellation'),
    ]:
        report_path = ROOT / 'playground/coroutine-implementation' / folder / 'report.json'
        report = json.loads(report_path.read_text())
        for backend in ('native', 'wasm'):
            case = report['results'][f'{backend}-restored-caught-empty-repeat-stale']
            row = next(r for r in case['evidence'] if r.get('snapshot', '').endswith('/cut-1.bin'))
            path = Path(row['snapshot'])
            assert sha(path) == row['sha256']
            assert path.read_bytes()[:8] == b'MONTY\0' + version.to_bytes(2, 'little')
            producer = next(r for r in case['evidence'] if r.get('argv', [])[-1:] == [str(path)])
            assert producer['exit'] == 0
            assert json.loads(producer['stdout']) == {'effect': {'phase': 'finally', 'trace': [['caught', '']]}}
            source = path.parent / 'guest.py'
            assert sha(source) == case['loaded_source_sha256']
            selected.append(
                {
                    'version': version,
                    'backend': backend,
                    'snapshot': str(path),
                    'sha256': sha(path),
                    'source_sha256': sha(source),
                    'report': str(report_path),
                    'report_sha256': sha(report_path),
                    'producer': producer,
                }
            )
    # Expectations and input provenance are sealed before any candidate execution.
    frozen = {
        'inputs': selected,
        'batches': ['"clean"', '{"native_cancel":"REPEAT","native_results":[{"reply":"MUST-NOT-APPLY"}]}'],
        'expected': {'exit_nonzero': True, 'stdout': '', 'stderr_contains': REFUSAL, 'output_exists': False},
    }
    (OUT / 'legacy-frozen.json').write_text(json.dumps(frozen, indent=2) + '\n')
    receipts = []
    for row in selected:
        command = (
            [str(ROOT / 'playground/coroutine-baseline/target/debug/native1')]
            if row['backend'] == 'native'
            else ['node', str(ROOT / 'tools/coroutines/native1/wasm.mjs')]
        )
        for ordinal, batch in enumerate(frozen['batches']):
            output = OUT / f'legacy-{row["version"]}-{row["backend"]}-{ordinal}.bin'
            args = command + ['resume', row['snapshot'], batch, str(output)]
            process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            stdout, stderr = process.communicate(timeout=60)
            passed = process.returncode != 0 and stdout == '' and REFUSAL in stderr and not output.exists()
            receipts.append(
                {
                    'input': row,
                    'argv': args,
                    'pid': process.pid,
                    'exit': process.returncode,
                    'stdout': stdout,
                    'stderr': stderr,
                    'passed': passed,
                }
            )
            assert sha(Path(row['snapshot'])) == row['sha256']
    (OUT / 'legacy-receipts.json').write_text(json.dumps(receipts, indent=2) + '\n')
    print(json.dumps({'passed': sum(r['passed'] for r in receipts), 'total': len(receipts)}))
    assert all(r['passed'] for r in receipts)


if __name__ == '__main__':
    main()
