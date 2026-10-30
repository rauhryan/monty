"""Milestone3 actual scope support; historical refusal contract stays immutable."""

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


def frozen(gate):
    for name, pin in gate.strict_json((ROOT / 'contract.sha256.json').read_text()).items():
        gate.check(gate.digest(ROOT / name) == pin, 'scope contract changed: ' + name)
    original = module('feasibility', ROOT.parent / 'anyio_feasibility/verify.py').frozen(gate)
    contract = gate.strict_json((ROOT / 'contract.json').read_text())
    gate.check(gate.equal(contract['scope_targets'], original['scope_targets']), 'upstream traces changed')
    for name in contract['scope_targets']:
        gate.check(
            (ROOT / 'cases' / f'{name}.py').read_bytes()
            == (ROOT.parent / 'anyio_feasibility/cases' / f'{name}.py').read_bytes(),
            'original source changed',
        )
    return contract, original


def run(output, association=None, backends=('native', 'wasm')):
    gate = module('gate', ROOT.parent / 'native_async/verify.py')
    steps = module('steps', ROOT.parent / 'native_async_queues/steps.py')
    groups = module('groups', ROOT.parent / 'native_async_groups/verify.py')
    gate.self_check()
    groups.self_check(gate)
    contract, old = frozen(gate)
    if association:
        receipt = gate.association(association)
        for path in ROOT.rglob('*'):
            if path.is_file() and '__pycache__' not in str(path):
                gate.check(
                    receipt['files'].get(str(path.relative_to(gate.REPO))) == gate.digest(path), 'unbound scope input'
                )
    output.mkdir(parents=True, exist_ok=False)
    (output / 'wasm.mjs').symlink_to(gate.ROOT / 'wasm.mjs')
    (output / 'cases').mkdir()
    cases = {**contract['scope_targets'], **contract['additional'], **old['supported'], **old['native_seams']}
    for name in cases:
        source = ROOT / 'cases' / f'{name}.py'
        if not source.exists():
            source = ROOT.parent / 'anyio_feasibility/cases' / f'{name}.py'
        (output / 'cases' / f'{name}.py').write_bytes(source.read_bytes())
    report = {
        'scope_targets': {},
        'additional': {},
        'supported': {},
        'native_seams': {},
        'budget_before': gate.budget(),
    }
    for backend in backends:
        for name, case in cases.items():
            d = output / (backend + '-' + name)
            result = steps.run_case(gate, output, name, case, backend, d)
            if result['status'] == 'PASS' and 'cancelled' in case:
                try:
                    groups.check_evidence(gate.strict_json((d / 'evidence.json').read_text()), 'host-cancel', gate)
                except (ValueError, TypeError, KeyError) as error:
                    result = {'status': 'FAIL', 'reason': str(error)}
            section = next(
                k
                for k in ('scope_targets', 'additional', 'supported', 'native_seams')
                if name in (contract | old).get(k, {})
            )
            report[section][backend + '/' + name] = result
            print(backend, name, result, flush=True)
    frozen(gate)
    if association:
        gate.association(association)
        report['association_sha256'] = gate.digest(association)
    report['budget_after'] = gate.budget()
    report['status'] = (
        'PASS'
        if all(
            v['status'] == 'PASS'
            for section in ('scope_targets', 'additional', 'supported', 'native_seams')
            for v in report[section].values()
        )
        else 'FAIL'
    )
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--association', type=Path, required=True)
    a = p.parse_args()
    return int(run(a.output.resolve(), a.association)['status'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
