"""Literal policy oracle only: no VM imports, execution, builds, or expectation updates."""

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path

CASES = Path(__file__).with_name('cases.json')


# Frozen public ExcType spellings; never infer valid types from candidate output.
EXCEPTION_TYPES = frozenset(
    """
Exception BaseException SystemExit KeyboardInterrupt ArithmeticError OverflowError
ZeroDivisionError LookupError IndexError KeyError RuntimeError NotImplementedError
RecursionError AttributeError FrozenInstanceError NameError UnboundLocalError ValueError
UnicodeDecodeError UnicodeEncodeError json.JSONDecodeError tomllib.TOMLDecodeError
ImportError ModuleNotFoundError OSError FileNotFoundError FileExistsError IsADirectoryError
NotADirectoryError PermissionError io.UnsupportedOperation TimeoutError AssertionError
MemoryError StopIteration SyntaxError TypeError re.PatternError binascii.Error
binascii.Incomplete _csv.Error
""".split()
)


def object_pairs(pairs):
    """Reject duplicate keys before a dict can erase them."""
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'Duplicate JSON key: {key}')
        result[key] = value
    return result


def reject_constant(value):
    """Reject JavaScript constants outside JSON."""
    raise ValueError(f'Invalid JSON constant: {value}')


def loads(raw):
    """Load strict JSON, including nested duplicate detection and finite numbers."""
    value = json.loads(raw, object_pairs_hook=object_pairs, parse_constant=reject_constant)
    json_value(value)
    return value


def json_value(value):
    """Validate Python inputs too; bool is deliberately distinct from int."""
    kind = type(value)
    if kind is dict:
        for key, child in value.items():
            if type(key) is not str:
                raise ValueError('JSON object key must be text')
            json_value(key)
            json_value(child)
    elif kind is list:
        for child in value:
            json_value(child)
    elif kind is str:
        try:
            value.encode('utf-8')
        except UnicodeEncodeError as error:
            raise ValueError('Invalid UTF-8 string') from error
    elif kind is float:
        if not math.isfinite(value):
            raise ValueError('Nonfinite JSON number')
    elif kind not in (int, bool, type(None)):
        raise ValueError(f'Not a JSON type: {kind}')


def keys(value, required):
    """Require an object with exactly the specified fields."""
    if type(value) is not dict or set(value) != set(required):
        raise ValueError(f'Expected exact object keys: {required}')


def summary(value):
    """Validate the bounded metadata schema independently of literal equality."""
    if value is None:
        return
    keys(value, ('type', 'message', 'message_truncated', 'representation'))
    if type(value['type']) is not str or value['type'] not in EXCEPTION_TYPES:
        raise ValueError('Invalid exception type')
    if value['representation'] != 'metadata-only' or type(value['representation']) is not str:
        raise ValueError('Invalid representation')
    if type(value['message_truncated']) is not bool:
        raise ValueError('message_truncated must be bool')
    message = value['message']
    if message is not None and type(message) is not str:
        raise ValueError('message must be string or null')
    if message is None and value['message_truncated']:
        raise ValueError('Null message cannot be truncated')
    if message is not None and len(message.encode('utf-8')) > 4096:
        raise ValueError('Message exceeds UTF-8 byte budget')


def observed(value):
    """Validate fixed envelopes while leaving guest values to the literal oracle."""
    json_value(value)
    keys(value, ('effects', 'terminal'))
    if type(value['effects']) is not list:
        raise ValueError('effects must be a list')
    for effect in value['effects']:
        keys(effect, ('phase', 'trace'))
        if type(effect['phase']) is not str or type(effect['trace']) is not list:
            raise ValueError('Invalid effect shape')
    terminal = value['terminal']
    keys(terminal, ('status', 'reason', 'value', 'error', 'interrupted_error', 'value_export_error'))
    if terminal['status'] != 'cancelled' or type(terminal['reason']) is not str:
        raise ValueError('Invalid cancelled status/reason')
    if terminal['error'] is not None and type(terminal['error']) is not str:
        raise ValueError('error must be string or null')
    export = terminal['value_export_error']
    if export is not None and (type(export) is not str or export not in ('unsupported-value', 'export-depth-limit')):
        raise ValueError('Invalid export error')
    if (export is not None and (terminal['value'] is not None or terminal['error'] is not None)) or (
        terminal['error'] is not None and terminal['value'] is not None
    ):
        raise ValueError('Inconsistent terminal payload')
    summary(terminal['interrupted_error'])


def exact(actual, expected):
    """Compare recursive types, keys and values without Python numeric coercion."""
    if type(actual) is not type(expected):
        return False
    if type(expected) is dict:
        return actual.keys() == expected.keys() and all(exact(actual[k], expected[k]) for k in expected)
    if type(expected) is list:
        return len(actual) == len(expected) and all(exact(a, e) for a, e in zip(actual, expected))
    return actual == expected


def expected_records(document):
    """Expand literal expectations across required transport/continuation coordinates."""
    return [
        {'case': case['id'], 'backend': backend, 'mode': mode, 'observed': case['expected']}
        for case in document['cases']
        for backend in ('native', 'wasm')
        for mode in ('uninterrupted', 'restored')
    ]


def check(document, records):
    """Require exact coverage and equality, including absent versus null fields."""
    expected = {(r['case'], r['backend'], r['mode']): r['observed'] for r in expected_records(document)}
    if type(records) is not list:
        raise ValueError('Records must be a list')
    for value in expected.values():
        observed(value)
    seen = set()
    for record in records:
        keys(record, ('case', 'backend', 'mode', 'observed'))
        if any(type(record[k]) is not str for k in ('case', 'backend', 'mode')):
            raise ValueError('Coordinates must be strings')
        observed(record['observed'])
        key = (record['case'], record['backend'], record['mode'])
        if key not in expected or key in seen:
            raise ValueError(f'Unknown or duplicate coordinate: {key}')
        seen.add(key)
        if not exact(record['observed'], expected[key]):
            raise ValueError(
                f'Literal contract mismatch: {key}\nexpected={expected[key]!r}\nactual={record["observed"]!r}'
            )
    if seen != set(expected):
        raise ValueError(f'Missing coordinates: {sorted(set(expected) - seen)}')


def self_check(document):
    """Check schema and oracle rejection; this does not test runtime behavior."""
    ids = [case['id'] for case in document['cases']]
    assert len(ids) == len(set(ids))
    for case in document['cases']:
        compile(case['source'], case['id'], 'exec')
        if 'historical_source' in case:
            origin = CASES.resolve().parents[3] / case['historical_source']
            raw = origin.read_bytes()
            assert hashlib.sha256(raw).hexdigest() == case['historical_source_sha256']
            assert raw.decode() == case['source']
            assert loads(origin.with_name('batches.json').read_text()) == case['batches']
        assert len(case['batches']) == len(case['expected']['effects'])
        assert set(case['expected']['terminal']) == {
            'status',
            'reason',
            'value',
            'error',
            'interrupted_error',
            'value_export_error',
        }
    records = expected_records(document)
    check(document, records)
    mutations = [records[:-1], records + [records[0]]]
    for field in ('status', 'reason', 'interrupted_error', 'value_export_error'):
        mutated = copy.deepcopy(records)
        mutated[0]['observed']['terminal'][field] = 'WRONG'
        mutations.append(mutated)
    mutated = copy.deepcopy(records)
    mutated[0]['observed']['effects'].append({'phase': 'purchase', 'trace': []})
    mutations.append(mutated)
    for mutated in mutations:
        try:
            check(document, mutated)
        except ValueError:
            continue
        raise AssertionError('Oracle accepted a deliberately incorrect observation')
    print(
        f'PASS contract self-check: {len(ids)} literal cases, {len(records)} coordinates, {len(mutations)} rejections; NO VM execution'
    )


def main():
    """Emit immutable inputs or check an independently collected observation file."""
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--self-check', action='store_true')
    action.add_argument('--emit-inputs', action='store_true')
    action.add_argument('--check', type=Path)
    args = parser.parse_args()
    document = loads(CASES.read_text())
    if args.self_check:
        self_check(document)
    elif args.emit_inputs:
        print(json.dumps([{k: c[k] for k in ('id', 'source', 'batches')} for c in document['cases']], indent=2))
    else:
        check(document, loads(args.check.read_text()))
        print('PASS literal observations; provenance and ownership gates remain separate')


if __name__ == '__main__':
    main()
