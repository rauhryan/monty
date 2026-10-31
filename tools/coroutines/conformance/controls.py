"""Focused repair controls: actual FIFO candidate and verifier-only receipt replay."""

import argparse
import json
import subprocess
import sys
from pathlib import Path

from adapter import ROOT
from run import CASES, decode, digest, equal, validate


def main():
    """Retain all control commands and outputs in a fresh evidence directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--fifo', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm'):
        for execution in ('uninterrupted', 'restored'):
            label = f'fifo-{backend}-{execution}'
            request = {
                'version': 1,
                'case': {k: v for k, v in CASES['drain'].items() if k != 'expected'},
                'backend': backend,
                'execution': execution,
                'output': str((args.output / label).resolve()),
                'source_directory': str(args.fifo.resolve()),
            }
            process = invoke([sys.executable, str(ROOT / 'adapter.py')], args.output / label, request)
            response = decode(process)
            # Require a completed interpreter run, with the specific reversed cleanup request.
            equal(response['views'][-1]['events'][5], ['request', 'O3', ['close', 'parent']])
            try:
                validate(CASES['drain'], response)
            except ValueError as error:
                results[label] = {'status': 'REJECT', 'error': str(error)}
            else:
                raise ValueError('FIFO candidate accepted')
    for mutation in ('unchanged', 'candidate-command', 'reference-bytes'):
        results[mutation] = receipt_control(args.output / mutation, args.baseline.resolve(), mutation)
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


def invoke(command, output, request=None):
    """Keep exact argv and raw process output, including expected rejection diagnostics."""
    process = subprocess.run(
        command, input=json.dumps(request) if request else None, capture_output=True, text=True, timeout=180
    )
    output.with_suffix('.json').write_text(
        json.dumps(
            {
                'command': command,
                'request': request,
                'exit': process.returncode,
                'stdout': process.stdout,
                'stderr': process.stderr,
            },
            indent=2,
        )
        + '\n'
    )
    return process


def receipt_control(directory, baseline, mutation):
    """Run the complete verifier with temporary receipts; replay is not interpreter evidence."""
    directory = directory.resolve()
    directory.mkdir()
    reference = directory / 'reference.json'
    reference.write_bytes((ROOT / 'binding.json').read_bytes())
    candidate = directory / 'candidate.json'
    mock = directory / 'mock.py'
    mock.write_text(
        'import json, sys\nfrom pathlib import Path\n'
        'r = json.load(sys.stdin)\n'
        "assert set(r['case']) == {'mode', 'batches'}\n"
        f'candidate = Path({str(candidate)!r})\nreference = Path({str(reference)!r})\n'
        f'if {mutation!r} == "candidate-command":\n'
        '    data = json.loads(candidate.read_text())\n'
        '    data["command"] = ["changed-after-start"]\n'
        '    candidate.write_text(json.dumps(data))\n'
        f'if {mutation!r} == "reference-bytes":\n'
        '    reference.write_bytes(reference.read_bytes() + b" ")\n'
        f'baseline = Path({str(baseline)!r})\n'
        'name = r["case"]["mode"] + "-" + r["backend"] + "-" + r["execution"] + ".json"\n'
        'print(json.loads((baseline / name).read_text())["stdout"])\n'
    )
    build = directory / 'mock-build.json'
    build.write_text('{"verifier_control_only": true}\n')
    lock = json.loads(reference.read_text())
    command = [sys.executable, str(mock)]
    candidate.write_text(
        json.dumps(
            {
                'command': command,
                'files': {str(p): digest(p) for p in (Path(sys.executable).resolve(), mock, build)},
                'build_receipt': str(build),
                'runtime_sha256': lock['runtime_sha256'],
                'build_inputs': lock['build_inputs'],
            },
            indent=2,
        )
    )
    before = {str(p): digest(p) for p in (reference, candidate)}
    for p in (reference, candidate):
        p.with_suffix('.before.json').write_bytes(p.read_bytes())
    wrapper = directory / 'runner.py'
    wrapper.write_text(
        f'import sys\nfrom pathlib import Path\nsys.path.insert(0, {str(ROOT)!r})\n'
        f'import run\nrun.main(reference_binding=Path({str(reference)!r}))\n'
    )
    process = invoke(
        [
            sys.executable,
            str(wrapper),
            '--adapter',
            json.dumps(command),
            '--candidate-binding',
            str(candidate),
            '--output',
            str(directory / 'run'),
        ],
        directory / 'process',
    )
    report = json.loads((directory / 'run/report.json').read_text())
    equal(report['cases'], {name: {'native': 'PASS', 'wasm': 'PASS'} for name in CASES})
    equal(len(report['negative_controls']), 12)
    if mutation == 'unchanged':
        equal(process.returncode, 0)
        equal(process.stderr, '')
        equal(report['status'], 'PASS')
    else:
        equal(process.returncode, 1)
        changed = candidate if mutation == 'candidate-command' else reference
        equal(process.stderr.splitlines()[-1], 'ValueError: receipt changed during run: ' + str(changed))
        equal(report['status'], 'FAIL')
    return {'status': report['status'], 'before': before, 'after': {str(p): digest(p) for p in (reference, candidate)}}


if __name__ == '__main__':
    main()
