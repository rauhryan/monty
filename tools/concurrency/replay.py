"""Strict transport comparison and genuine restore/disposal checks."""

import hashlib
import json
import math
import subprocess


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


def invoke(command, arguments, evidence):
    result = subprocess.run([*command, *map(str, arguments)], capture_output=True, text=True, timeout=40)
    evidence.append(
        {
            'argv': result.args,
            'exit': result.returncode,
            'stdout': result.stdout,
            'stderr': result.stderr,
        }
    )
    check(
        result.returncode == 0 and not result.stderr,
        'adapter failed: ' + (result.stderr or result.stdout)[-1200:],
    )
    return strict_json(result.stdout)


def view(raw, identities):
    if 'complete' in raw:
        check(set(raw) == {'complete'}, 'unexpected terminal fields')
        return raw
    if 'call' in raw:
        check(set(raw) == {'call'}, 'invalid active call boundary')
        return ['call', register_call(raw['call'], identities)]
    check(set(raw) == {'calls', 'pending'}, 'invalid boundary')
    labels = []
    for call in raw['calls']:
        labels.append(register_call(call, identities))
    check(
        all(type(i) is int and i in identities for i in raw['pending']),
        'unknown pending ID',
    )
    check(len(set(raw['pending'])) == len(raw['pending']), 'duplicate pending ID')
    return [labels, sorted(identities[i] for i in raw['pending'])]


def register_call(call, identities):
    check(
        set(call) == {'id', 'label'} and type(call['id']) is int and type(call['label']) is str,
        'invalid call',
    )
    check(
        call['id'] not in identities and call['label'] not in identities.values(),
        'aliased call',
    )
    identities[call['id']] = call['label']
    return call['label']


def terminal(raw, case):
    check(set(raw) == {'complete'}, 'did not complete')
    if 'terminal' in case:
        check(equal(raw['complete'], case['terminal']), 'cancellation terminal changed')
    elif 'cancelled' in case:
        value = raw['complete']
        check(
            type(value) is dict
            and value.get('status') == 'cancelled'
            and equal(value.get('reason'), case['cancelled']),
            'lost sticky host cancellation',
        )
        # Diagnostic policy is outside this narrow case; preserve all raw fields.
    else:
        check(equal(raw['complete'], case['result']), 'wrong final value: ' + repr(raw))


def run_case(case, command, source, directory):
    directory.mkdir()
    evidence = []
    snapshots = []
    suffix = '-step' if case.get('step', False) else ''
    try:
        snapshot = directory / 'cut-0.bin'
        raw = invoke(command, ['start' + suffix, source, snapshot], evidence)
        identities = {}
        restored = [raw]
        batches = []
        for index, (step, scripted) in enumerate(zip(case['steps'], case['batches'])):
            check(
                equal(view(raw, identities), step),
                f'boundary {index}: {raw!r} != {step!r}',
            )
            check(snapshot.is_file() and snapshot.stat().st_size > 0, 'missing snapshot')
            original = digest(snapshot)
            snapshots.append({'path': str(snapshot), 'sha256': original})
            check(
                equal(invoke(command, ['dispose', snapshot], evidence), {'disposed': True}),
                'disposal failed',
            )
            by_label = {label: ident for ident, label in identities.items()}
            batch = {
                'results': [
                    {'id': by_label[label], 'value': value}
                    for label, value in scripted.items()
                    if label not in ('$cancel', '$ack')
                ]
            }
            if '$ack' in scripted:
                batch = {'ack': True}
            if '$cancel' in scripted:
                batch['cancel'] = scripted['$cancel']
            batches.append(batch)
            target = directory / f'cut-{index + 1}.bin'
            raw = invoke(command, ['resume' + suffix, snapshot, json.dumps(batch), target], evidence)
            branch = directory / f'branch-{index}.bin'
            second = invoke(command, ['resume' + suffix, snapshot, json.dumps(batch), branch], evidence)
            check(
                equal(raw, second) and digest(snapshot) == original,
                'restore branching mismatch',
            )
            if branch.exists():
                check(
                    equal(
                        invoke(command, ['dispose', branch], evidence),
                        {'disposed': True},
                    ),
                    'branch disposal failed',
                )
            restored.append(raw)
            snapshot = target
        terminal(raw, case)
        batch_file = directory / 'batches.json'
        batch_file.write_text(json.dumps(batches))
        continuous = invoke(command, ['run' + suffix, source, batch_file], evidence)
        check(equal(continuous, restored), 'uninterrupted versus restored mismatch')
        return {'status': 'PASS', 'cuts': len(snapshots)}
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        return {'status': 'FAIL', 'reason': str(error)}
    finally:
        (directory / 'evidence.json').write_text(
            json.dumps({'commands': evidence, 'snapshots': snapshots}, indent=2) + '\n'
        )


def self_check():
    for actual, expected in [
        (False, 0),
        (0.0, 0),
        ([1, 2], [2, 1]),
        ({'a': 1}, {}),
        ([], ['release']),
    ]:
        check(not equal(actual, expected), 'false-pass comparator')
    for text in ('{"a":1,"a":2}', 'NaN', '1e999'):
        try:
            strict_json(text)
        except ValueError:
            continue
        raise ValueError('invalid JSON accepted')
    for bad in (
        {'complete': {'status': 'completed', 'reason': ''}},
        {'complete': {'status': 'cancelled', 'reason': 'SECOND'}},
    ):
        try:
            terminal(bad, {'cancelled': ''})
        except ValueError:
            continue
        raise ValueError('cancellation mutation accepted')
    print('PASS: fixed inputs and 10 negative controls')
