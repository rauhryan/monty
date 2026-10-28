"""Reexecute frozen current-error/S precedence controls with independent restored suffixes."""

import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'playground/coroutine-implementation/catchable-cancellation-compatibility11'
INPUTS = ROOT / 'playground/coroutine-implementation/catchable-cancellation9/inputs'


def main():
    cases = [
        c
        for c in json.loads((INPUTS / 'supplemental.json').read_text())
        if c['id'] in ('current-over-S', 'handled-outer')
    ]
    (OUT / 'diagnostics-frozen.json').write_text(json.dumps(cases, indent=2) + '\n')
    receipts = []
    for backend in ('native', 'wasm'):
        command = (
            [str(ROOT / 'playground/coroutine-baseline/target/debug/native1')]
            if backend == 'native'
            else ['node', str(ROOT / 'tools/coroutines/native1/wasm.mjs')]
        )

        def invoke(args):
            process = subprocess.Popen(
                command + list(map(str, args)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
            )
            stdout, stderr = process.communicate(timeout=60)
            receipts.append(
                {
                    'pid': process.pid,
                    'argv': process.args,
                    'exit': process.returncode,
                    'stdout': stdout,
                    'stderr': stderr,
                }
            )
            assert process.returncode == 0 and not stderr
            return json.loads(stdout)

        for case in cases:
            source = INPUTS / case['id'] / 'guest.py'
            assert source.read_text() == case['source']
            snapshot = OUT / f'diagnostic-{backend}-{case["id"]}.bin'
            initial = invoke(['start', source, snapshot])
            assert initial['effect'] == case['expected']['effects'][0]
            before = snapshot.read_bytes()
            invoke(['dispose', snapshot])
            for branch in range(2):
                result = invoke(
                    [
                        'resume',
                        snapshot,
                        json.dumps(case['batches'][0]),
                        OUT / f'unused-{backend}-{case["id"]}-{branch}.bin',
                    ]
                )
                terminal = result['complete']
                terminal['value'] = json.loads(terminal['value'])
                assert terminal == case['expected']['terminal']
                assert snapshot.read_bytes() == before
            receipts.append(
                {
                    'snapshot': str(snapshot),
                    'sha256': hashlib.sha256(before).hexdigest(),
                    'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                    'branches': 2,
                }
            )
    (OUT / 'diagnostics-receipts.json').write_text(json.dumps(receipts, indent=2) + '\n')
    print('PASS: four native/Wasm cuts; eight independent restored suffixes')


if __name__ == '__main__':
    main()
