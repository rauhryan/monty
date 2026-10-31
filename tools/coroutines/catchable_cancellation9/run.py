"""Execute frozen inputs with raw process receipts; expectations never come from the VM."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / 'playground/coroutine-implementation/catchable-cancellation9'
INPUTS = EVIDENCE / 'inputs'
parser = argparse.ArgumentParser()
parser.add_argument('backend', choices=['native', 'wasm'])
parser.add_argument('output', type=Path)
args = parser.parse_args()
args.output.mkdir(exist_ok=False, parents=True)
command = (
    [str(ROOT / 'playground/coroutine-baseline/target/debug/native1')]
    if args.backend == 'native'
    else ['node', str(ROOT / 'tools/coroutines/native1/wasm.mjs')]
)
cases = json.loads((ROOT / 'tools/coroutines/catchable_cancellation7/cases.json').read_text())['cases']
cases += json.loads((INPUTS / 'supplemental.json').read_text())
records = []
failures = []
for case in cases:
    directory = INPUTS / case['id']
    process = subprocess.Popen(
        command + ['run', str(directory / 'guest.py'), str(directory / 'batches.json')],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    stdout, stderr = process.communicate(timeout=60)
    receipt = {
        'pid': process.pid,
        'argv': process.args,
        'exit': process.returncode,
        'stdout': stdout,
        'stderr': stderr,
        'source_sha256': hashlib.sha256((directory / 'guest.py').read_bytes()).hexdigest(),
        'batches_sha256': hashlib.sha256((directory / 'batches.json').read_bytes()).hexdigest(),
    }
    try:
        assert process.returncode == 0 and not stderr, 'process failure'
        result = json.loads(stdout)
        terminal = result['views'][-1]
        if terminal['value'] is not None:
            terminal['value'] = json.loads(terminal['value'])
        if terminal['error'] is not None:
            terminal['error'] = terminal['error'].strip().splitlines()[-1]
        observed = {'effects': result['views'][:-1], 'terminal': terminal}
        assert observed == case['expected'], {'actual': observed, 'expected': case['expected']}
        records.append({'case': case['id'], 'backend': args.backend, 'mode': 'uninterrupted', 'observed': observed})
        receipt['status'] = 'PASS'
    except Exception as error:
        receipt['status'] = 'FAIL'
        receipt['failure'] = str(error)
        failures.append(case['id'])
    (args.output / (case['id'] + '.json')).write_text(json.dumps(receipt, indent=2, ensure_ascii=False) + '\n')
(args.output / 'records.json').write_text(json.dumps(records, indent=2, ensure_ascii=False) + '\n')
(args.output / 'report.json').write_text(json.dumps({'passed': len(records), 'failed': failures}, indent=2) + '\n')
print(json.dumps({'passed': len(records), 'failed': failures}))
raise SystemExit(bool(failures))
