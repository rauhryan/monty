"""Additive group-entry checkpoint regressions; original contracts stay frozen."""

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def run(output, association):
    gate = module('gate', ROOT.parent / 'native_async/verify.py')
    steps = module('steps', ROOT.parent / 'native_async_queues/steps.py')
    groups = module('groups', ROOT.parent / 'native_async_groups/verify.py')
    gate.self_check()
    groups.self_check(gate)
    receipt = gate.association(association)
    pins = gate.strict_json((ROOT / 'contract.sha256.json').read_text())
    for name, pin in pins.items():
        gate.check(gate.digest(ROOT / name) == pin, 'changed additive input: ' + name)
    for path in ROOT.rglob('*'):
        if path.is_file() and '__pycache__' not in str(path):
            gate.check(receipt['files'].get(str(path.relative_to(gate.REPO))) == gate.digest(path), 'unbound input')
    original = gate.REPO / 'playground/coroutine-implementation/anyio-scopes-review1/checkpoint-bypass'
    for name in ('group', 'native-group', 'list', 'scope'):
        gate.check(
            (ROOT / 'cases' / (name + '.py')).read_bytes() == (original / 'cases' / (name + '.py')).read_bytes(),
            'exact review source changed',
        )
    cases = gate.strict_json((ROOT / 'contract.json').read_text())
    output.mkdir(parents=True, exist_ok=False)
    (output / 'wasm.mjs').symlink_to(gate.ROOT / 'wasm.mjs')
    (output / 'cases').mkdir()
    for name in cases:
        (output / 'cases' / (name + '.py')).write_bytes((ROOT / 'cases' / (name + '.py')).read_bytes())
    report = {}
    for backend in ('native', 'wasm'):
        for name, case in cases.items():
            result = steps.run_case(gate, output, name, case, backend, output / (backend + '-' + name))
            report[backend + '/' + name] = result
            print(backend, name, result, flush=True)
    gate.association(association)
    for name, pin in pins.items():
        gate.check(gate.digest(ROOT / name) == pin, 'changed input after execution')
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return all(result['status'] == 'PASS' for result in report.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--association', type=Path, required=True)
    args = parser.parse_args()
    return int(not run(args.output.resolve(), args.association))


if __name__ == '__main__':
    raise SystemExit(main())
