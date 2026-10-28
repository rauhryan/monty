"""Targeted native slice proof; full migration acceptance remains a separate FAIL gate."""

import argparse
import importlib
import json
import sys
from pathlib import Path

from adapter import ROOT, execute, run

# Verification only; these modules are never imported by the candidate adapter.
del sys.modules['adapter']  # The verifier must bind its own fixed-reference adapter.
sys.path.insert(0, str(ROOT.parent / 'conformance'))
CASES = importlib.import_module('cases').CASES
verifier = importlib.import_module('run')
fixed_reference, validate = verifier.fixed_reference, verifier.validate


def main():
    """Assert literal semantics and real per-cut restored execution on each backend."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    body = (
        ROOT / 'semantics.py'
    ).read_text() + "\nvalue = semantic_body()._native1()\njson.dumps({'value': value, 'trace': TRACE})\n"
    for backend in ('native', 'wasm'):
        for execution in ('uninterrupted', 'restored'):
            key = backend + '-' + execution
            case = CASES['nested']
            result = run({'case': case, 'backend': backend, 'execution': execution, 'output': str(args.output / key)})
            validate(case, result)
            assert result['views'] == fixed_reference(case)['views']
            results[key] = result
            host_case = CASES['host']
            host_result = run(
                {
                    'case': host_case,
                    'backend': backend,
                    'execution': execution,
                    'output': str(args.output / (key + '-host')),
                }
            )
            validate(host_case, host_result)
            assert host_result['views'] == fixed_reference(host_case)['views']
            results[key + '-host'] = host_result
            for label, batches, value, trace in (
                (
                    'value',
                    ['read-ok', 'audited', 'closed'],
                    'read-ok',
                    [['read', 'read-ok'], ['audit', 'audited'], ['close', 'closed']],
                ),
                (
                    'cancel',
                    [{'raise': 'task-cancel:FIRST'}, {'raise': 'AUDIT'}, 'closed'],
                    'caught',
                    [['caught', 'task-cancel:FIRST'], ['audit-error', 'AUDIT'], ['close', 'closed']],
                ),
                (
                    'error',
                    [{'raise': 'IO'}, 'audited', 'closed'],
                    'caught',
                    [['caught', 'IO'], ['audit', 'audited'], ['close', 'closed']],
                ),
            ):
                result = execute(body, batches, backend, execution, args.output / (key + '-' + label))
                assert result['views'][-1] == {'value': value, 'trace': trace}, result
                assert [v['phase'] for v in result['views'][:-1]] == ['read', 'audit', 'close']
                results[key + '-' + label] = result
    for label in ('', '-host', '-value', '-cancel', '-error'):
        expected = results['native-uninterrupted' + label]['views']
        for key in ('native-restored', 'wasm-uninterrupted', 'wasm-restored'):
            assert results[key + label]['views'] == expected
    (args.output / 'report.json').write_text(
        json.dumps(
            {
                'status': 'PASS targeted slice only',
                'full_gate': 'FAIL: nine mandatory cases unsupported',
                'results': results,
            },
            indent=2,
        )
        + '\n'
    )
    print(
        'PASS: nested + sticky host + value/error/cancellation injection, native and actual Wasm, continuous/restored'
    )


if __name__ == '__main__':
    main()
