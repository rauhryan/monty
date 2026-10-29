"""Fixed native-asyncio behavioral score. Never updates expectations or builds."""
import argparse
import hashlib
import importlib.util
import json
import math
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
sys.path.insert(0, str(ROOT.parent / 'native1'))
from bind import inputs  # noqa: E402


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate JSON key')
            result[key] = value
        return result
    def number(value):
        raise ValueError('nonfinite JSON')
    def finite(value):
        parsed = float(value)
        if not math.isfinite(parsed):
            raise ValueError('nonfinite JSON')
        return parsed
    return json.loads(text, object_pairs_hook=pairs, parse_constant=number, parse_float=finite)


def equal(actual, expected):
    # Python == alone accepts False == 0 and 1 == 1.0.
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(equal(actual[k], v) for k, v in expected.items())
    if isinstance(expected, list):
        return len(actual) == len(expected) and all(equal(a, b) for a, b in zip(actual, expected))
    return actual == expected


def check(condition, message):
    if not condition:
        raise ValueError(message)


def frozen():
    binding = strict_json((ROOT / 'contract.sha256.json').read_text())
    for name, expected in binding.items():
        check(digest(ROOT / name) == expected, 'frozen input changed: ' + name)
    cases = strict_json((ROOT / 'contract.json').read_text())['cases']
    for name, case in cases.items():
        check(len(case['steps']) == len(case['batches']) > 0, 'invalid host script')
        source = (ROOT / 'cases' / (name + '.py')).read_text()
        check('__monty_host__' not in source and '._native' not in source, 'experimental guest entry')
    return cases


def budget():
    spec = importlib.util.spec_from_file_location('guard', ROOT.parent / 'coordinator1/guarded_checks.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    usage = module.usage()
    module.build.require_budget(usage)
    return usage


def association(path):
    receipt = strict_json(path.read_text())
    for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
        for name, expected in receipt[group].items():
            check(digest(REPO / name) == expected, 'stale/missing source or artifact: ' + name)
    current = inputs()
    for group in ('runtime', 'build_inputs'):
        check(equal(current[group], receipt[group]), 'source inventory changed: ' + group)
    for name in ('native.rs', 'wasm.mjs'):
        key = str((ROOT / name).relative_to(REPO))
        check(receipt['files'].get(key) == digest(ROOT / name), 'adapter not associated')
    key = 'playground/coroutine-baseline/target/debug/native_async'
    check(receipt['artifacts'].get(key) == digest(REPO / key), 'native adapter not associated')
    return receipt


def invoke(command, arguments, evidence):
    result = subprocess.run([*command, *map(str, arguments)], capture_output=True, text=True, timeout=40)
    evidence.append({'argv': result.args, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
    check(result.returncode == 0 and not result.stderr, 'adapter failed: ' + (result.stderr or result.stdout)[-1200:])
    return strict_json(result.stdout)


def view(raw, identities):
    if 'complete' in raw:
        check(set(raw) == {'complete'}, 'unexpected terminal fields')
        return raw
    check(set(raw) == {'calls', 'pending'}, 'invalid boundary')
    labels = []
    for call in raw['calls']:
        check(set(call) == {'id', 'label'} and type(call['id']) is int and type(call['label']) is str, 'invalid call')
        check(call['id'] not in identities and call['label'] not in identities.values(), 'aliased call')
        identities[call['id']] = call['label']
        labels.append(call['label'])
    check(all(type(i) is int and i in identities for i in raw['pending']), 'unknown pending ID')
    check(len(set(raw['pending'])) == len(raw['pending']), 'duplicate pending ID')
    return [labels, sorted(identities[i] for i in raw['pending'])]


def terminal(raw, case):
    check(set(raw) == {'complete'}, 'did not complete')
    if 'cancelled' in case:
        value = raw['complete']
        check(type(value) is dict and value.get('status') == 'cancelled' and equal(value.get('reason'), case['cancelled']), 'lost sticky host cancellation')
        # Diagnostic policy is outside this narrow case; preserve all raw fields.
    else:
        check(equal(raw['complete'], case['result']), 'wrong final value: ' + repr(raw))


def run_case(name, case, backend, directory):
    directory.mkdir()
    evidence = []
    command = [str(REPO / 'playground/coroutine-baseline/target/debug/native_async')] if backend == 'native' else ['node', str(ROOT / 'wasm.mjs')]
    source = ROOT / 'cases' / (name + '.py')
    snapshots = []
    try:
        snapshot = directory / 'cut-0.bin'
        raw = invoke(command, ['start', source, snapshot], evidence)
        identities = {}
        restored = [raw]
        batches = []
        for index, (step, scripted) in enumerate(zip(case['steps'], case['batches'])):
            check(equal(view(raw, identities), step), f'boundary {index}: {raw!r} != {step!r}')
            check(snapshot.is_file() and snapshot.stat().st_size > 0, 'missing snapshot')
            original = digest(snapshot)
            snapshots.append({'path': str(snapshot), 'sha256': original})
            check(equal(invoke(command, ['dispose', snapshot], evidence), {'disposed': True}), 'disposal failed')
            by_label = {label: ident for ident, label in identities.items()}
            batch = {'results': [{'id': by_label[label], 'value': value} for label, value in scripted.items() if label != '$cancel']}
            if '$cancel' in scripted:
                batch['cancel'] = scripted['$cancel']
            batches.append(batch)
            target = directory / f'cut-{index + 1}.bin'
            raw = invoke(command, ['resume', snapshot, json.dumps(batch), target], evidence)
            branch = directory / f'branch-{index}.bin'
            second = invoke(command, ['resume', snapshot, json.dumps(batch), branch], evidence)
            check(equal(raw, second) and digest(snapshot) == original, 'restore branching mismatch')
            if branch.exists():
                check(equal(invoke(command, ['dispose', branch], evidence), {'disposed': True}), 'branch disposal failed')
            restored.append(raw)
            snapshot = target
        terminal(raw, case)
        batch_file = directory / 'batches.json'
        batch_file.write_text(json.dumps(batches))
        continuous = invoke(command, ['run', source, batch_file], evidence)
        check(equal(continuous, restored), 'uninterrupted versus restored mismatch')
        return {'status': 'PASS', 'cuts': len(snapshots)}
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        return {'status': 'FAIL', 'reason': str(error)}
    finally:
        (directory / 'evidence.json').write_text(json.dumps({'commands': evidence, 'snapshots': snapshots}, indent=2) + '\n')


def self_check():
    frozen()
    for actual, expected in [(False, 0), (0.0, 0), ([1, 2], [2, 1]), ({'a': 1}, {}), ([], ['release'])]:
        check(not equal(actual, expected), 'false-pass comparator')
    for text in ('{"a":1,"a":2}', 'NaN', '1e999'):
        try:
            strict_json(text)
        except ValueError:
            continue
        raise ValueError('invalid JSON accepted')
    for bad in ({'complete': {'status': 'completed', 'reason': ''}}, {'complete': {'status': 'cancelled', 'reason': 'SECOND'}}):
        try:
            terminal(bad, {'cancelled': ''})
        except ValueError:
            continue
        raise ValueError('cancellation mutation accepted')
    print('PASS: fixed inputs and 10 negative controls')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--association', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--self-check', action='store_true')
    args = parser.parse_args()
    if args.self_check:
        self_check()
        return 0
    check(args.association is not None, '--association is required (run build.py first)')
    output = args.output or REPO / 'playground/coroutine-implementation' / ('native-async-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'))
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'FAIL', 'results': {}}
    try:
        cases = frozen()
        receipt_hash = digest(args.association)
        report['disk_before'] = budget()
        association(args.association)
        report['association_sha256'] = receipt_hash
        for backend in ('native', 'wasm'):
            for name, case in cases.items():
                result = run_case(name, case, backend, output / (backend + '-' + name))
                report['results'][backend + '/' + name] = result
                print(backend, name, result['status'], flush=True)
        frozen()
        association(args.association)
        check(digest(args.association) == receipt_hash, 'receipt changed during execution')
        report['disk_after'] = budget()
        passed = sum(r['status'] == 'PASS' for r in report['results'].values())
        report['score'] = {'passed': passed, 'required': len(cases) * 2}
        if passed == len(cases) * 2:
            report['status'] = 'PASS'
    except (ValueError, OSError, RuntimeError) as error:
        report['error'] = str(error)
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k in ('status', 'score', 'error')}))
    print(output / 'report.json')
    return int(report['status'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
