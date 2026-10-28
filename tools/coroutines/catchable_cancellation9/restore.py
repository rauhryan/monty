"""Enumerated same-byte restore branches with process/disposal receipts at every frozen cut."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
INPUTS = ROOT / 'playground/coroutine-implementation/catchable-cancellation9/inputs'
p = argparse.ArgumentParser()
p.add_argument('backend', choices=['native', 'wasm'])
p.add_argument('output', type=Path)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
command = (
    [str(ROOT / 'playground/coroutine-baseline/target/debug/native1')]
    if a.backend == 'native'
    else ['node', str(ROOT / 'tools/coroutines/native1/wasm.mjs')]
)
cases = json.loads((ROOT / 'tools/coroutines/catchable_cancellation7/cases.json').read_text())['cases']
cases += json.loads((INPUTS / 'supplemental.json').read_text())
records = []
rows = []
failures = []


def invoke(argv, receipts):
    process = subprocess.Popen(
        command + list(map(str, argv)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
    )
    stdout, stderr = process.communicate(timeout=60)
    receipts.append(
        {'pid': process.pid, 'argv': process.args, 'exit': process.returncode, 'stdout': stdout, 'stderr': stderr}
    )
    assert process.returncode == 0 and not stderr, receipts[-1]
    return json.loads(stdout)


def normalize(terminal):
    result = dict(terminal)
    if result['value'] is not None:
        result['value'] = json.loads(result['value'])
    if result['error'] is not None:
        result['error'] = result['error'].strip().splitlines()[-1]
    return result


for case in cases:
    directory = a.output / case['id']
    directory.mkdir()
    receipts = []
    case_rows = []
    try:
        source = INPUTS / case['id'] / 'guest.py'
        snapshot = directory / 'cut-0.bin'
        actual = invoke(['start', source, snapshot], receipts)
        effects = [actual['effect']]
        for cut, batch in enumerate(case['batches']):
            before = snapshot.read_bytes()
            row = {
                'case': case['id'],
                'backend': a.backend,
                'cut': cut,
                'snapshot': str(snapshot),
                'sha256': hashlib.sha256(before).hexdigest(),
                'header_hex': before[:8].hex(),
                'receipt_start': len(receipts),
                'pending_ids': actual.get('raw_pending_ids'),
                'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                'batches_sha256': hashlib.sha256((source.parent / 'batches.json').read_bytes()).hexdigest(),
            }
            # Additional immediate-disposal branch, independent of both continuation processes.
            assert invoke(['dispose', snapshot], receipts) == {'disposed': True}
            branches = []
            suffixes = []
            for branch in range(2):
                target = directory / f'cut-{cut}-branch-{branch}.bin'
                result = invoke(['resume', snapshot, json.dumps(batch), target], receipts)
                branches.append(result)
                if 'complete' not in result:
                    assert invoke(['dispose', target], receipts) == {'disposed': True}
                suffix = []
                cursor = target
                step = result
                last_effect = effects[-1]
                if 'complete' not in step:
                    last_effect = step.get('effect', last_effect)
                    suffix.append(last_effect)
                for next_cut, next_batch in enumerate(case['batches'][cut + 1 :], cut + 1):
                    next_target = directory / f'suffix-{cut}-{branch}-{next_cut}.bin'
                    step = invoke(['resume', cursor, json.dumps(next_batch), next_target], receipts)
                    if 'complete' not in step:
                        assert invoke(['dispose', next_target], receipts) == {'disposed': True}
                        last_effect = step.get('effect', last_effect)
                        suffix.append(last_effect)
                    cursor = next_target
                suffix_terminal = normalize(step['complete'])
                assert suffix == case['expected']['effects'][cut + 1 :]
                assert suffix_terminal == case['expected']['terminal']
                suffixes.append({'effects': suffix, 'terminal': suffix_terminal})
            assert suffixes[0] == suffixes[1]
            row['independent_complete_suffixes'] = len(suffixes)
            assert branches[0] == branches[1]
            assert snapshot.read_bytes() == before
            actual = branches[0]
            if 'complete' in actual:
                terminal = normalize(actual['complete'])
            else:
                effects.append(actual.get('effect', effects[-1]))
            row['receipt_end'] = len(receipts)
            case_rows.append(row)
            snapshot = directory / f'cut-{cut}-branch-0.bin'
        observed = {'effects': effects, 'terminal': terminal}
        assert observed == case['expected'], {'actual': observed, 'expected': case['expected']}
        records.append({'case': case['id'], 'backend': a.backend, 'mode': 'restored', 'observed': observed})
    except Exception as error:
        failures.append({'case': case['id'], 'error': str(error)})
    (directory / 'receipts.json').write_text(json.dumps(receipts, indent=2, ensure_ascii=False) + '\n')
    rows.extend(case_rows)
(a.output / 'rows.json').write_text(json.dumps(rows, indent=2) + '\n')
(a.output / 'records.json').write_text(json.dumps(records, indent=2, ensure_ascii=False) + '\n')
(a.output / 'report.json').write_text(json.dumps({'passed': len(records), 'failures': failures}, indent=2) + '\n')
print(json.dumps({'passed': len(records), 'failures': failures}))
raise SystemExit(bool(failures))
