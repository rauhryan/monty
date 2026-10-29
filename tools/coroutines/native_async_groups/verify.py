"""Additive structured TaskGroup and prerequisite lifetime traces; fixed gate unchanged."""

import argparse
import copy
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def terminals(value):
    """Visit fresh-restored and uninterrupted terminal outputs alike."""
    if isinstance(value, list):
        for item in value:
            yield from terminals(item)
    elif isinstance(value, dict) and 'complete' in value:
        yield value['complete']


def check_terminal(value, name, gate):
    """Require the complete error-channel contract, including exact aggregates."""
    gate.check(
        set(value) == {'status', 'reason', 'value', 'value_export_error', 'error', 'interrupted_error'},
        'unexpected terminal channels',
    )
    gate.check(value['status'] == 'cancelled' and value['reason'] == 'FIRST', 'lost host cancellation')
    gate.check(value['value'] is None and value['value_export_error'] is None, 'unexpected terminal value/export error')
    error = value['error']
    gate.check(
        isinstance(error, str)
        and [
            line
            for line in error.splitlines()
            if line and not line[0].isspace() and line != 'Traceback (most recent call last):'
        ]
        == ['ValueError: FIRST']
        and 'AssertionError' not in error,
        'unexpected primary error',
    )
    expected = None
    if name == 'failure-host':
        expected = {
            'type': 'RuntimeError',
            'message': "unhandled errors in a TaskGroup (ExceptionGroup unsupported): ValueError('first'); TypeError('second')",
            'message_truncated': False,
            'representation': 'metadata-only',
        }
    gate.check(gate.equal(value['interrupted_error'], expected), 'unexpected aggregate/error channels')


def check_evidence(evidence, name, gate):
    """Check every successful adapter terminal, including uninterrupted arrays."""
    values = []
    for command in evidence['commands']:
        if command['exit'] == 0:
            values.extend(terminals(gate.strict_json(command['stdout'])))
    gate.check(bool(values), 'missing terminal cancellation diagnostics')
    for value in values:
        check_terminal(value, name, gate)


def self_check(gate):
    """Executable controls reject extra, duplicate, missing and hidden failures."""
    good = {
        'status': 'cancelled',
        'reason': 'FIRST',
        'value': None,
        'value_export_error': None,
        'error': 'ValueError: FIRST',
        'interrupted_error': {
            'type': 'RuntimeError',
            'message': "unhandled errors in a TaskGroup (ExceptionGroup unsupported): ValueError('first'); TypeError('second')",
            'message_truncated': False,
            'representation': 'metadata-only',
        },
    }

    def evidence(value):
        return {'commands': [{'exit': 0, 'stdout': json.dumps([[{'complete': value}]])}]}

    check_evidence(evidence(good), 'failure-host', gate)
    normal = copy.deepcopy(good)
    normal['interrupted_error'] = None
    check_evidence(evidence(normal), 'host-cancel', gate)
    bad = []
    for suffix in (
        "; AssertionError('unexpected body reached')",
        "; ValueError('first')",
        "; TypeError('second')",
        "; ValueError('extra')",
    ):
        value = copy.deepcopy(good)
        value['interrupted_error']['message'] += suffix
        bad.append((value, 'failure-host'))
    for field, replacement in (
        ('message', 'first second'),
        ('type', 'AssertionError'),
        ('message_truncated', True),
        ('representation', 'other'),
    ):
        value = copy.deepcopy(good)
        value['interrupted_error'][field] = replacement
        bad.append((value, 'failure-host'))
    for field, replacement in (
        ('error', 'AssertionError: body reached'),
        ('error', 'RuntimeError: unexpected\nValueError: FIRST'),
        ('error', 'ValueError: FIRST\nValueError: FIRST'),
        ('value_export_error', 'AssertionError: export'),
        ('value', 'unexpected'),
        ('interrupted_error', None),
    ):
        value = copy.deepcopy(good)
        value[field] = replacement
        bad.append((value, 'failure-host'))
    bad.append((good, 'host-cancel'))
    for value, name in bad:
        try:
            check_evidence(evidence(value), name, gate)
        except (ValueError, KeyError, TypeError):
            pass
        else:
            raise AssertionError('negative control accepted')
    print(f'{len(bad)} additive negative controls PASS')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--association', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--self-check', action='store_true')
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('gate', ROOT.parent / 'native_async/verify.py')
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    if args.self_check:
        self_check(gate)
        return 0
    if args.association is None or args.output is None:
        parser.error('--association and --output are required')
    gate.association(args.association)
    gate.frozen()
    before = gate.budget()
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'cases').mkdir()
    (args.output / 'wasm.mjs').symlink_to(gate.ROOT / 'wasm.mjs')
    programs = {
        'body-gather': {
            'steps': [
                [['K', 'A', 'B'], ['A', 'B', 'K']],
                [['K-release', 'B-release'], ['B-release', 'K-release']],
                [[], ['K-release']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'A': 'a'},
                {'B-release': 'b'},
                {'K-release': 'k'},
                {'parent-release': 'p'},
            ],
            'result': 'ok',
        },
        'nested-caught': {
            'steps': [
                [['body', 'O', 'A', 'B'], ['A', 'B', 'O', 'body']],
                [['child-release'], ['O', 'child-release']],
                [['after-inner'], ['O', 'after-inner']],
                [[], ['O']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'A': 'a'},
                {'child-release': 'r'},
                {'after-inner': 'i'},
                {'O': 'o'},
                {'parent-release': 'p'},
            ],
            'result': 'ok',
        },
        'same-owner': {
            'steps': [
                [['A', 'B'], ['A', 'B']],
                [['after-inner'], ['A', 'after-inner']],
                [[], ['A']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'B': 'b'},
                {'after-inner': 'i'},
                {'A': 'a'},
                {'parent-release': 'p'},
            ],
            'result': 'ok',
        },
        'protected': {
            'steps': [
                [['body', 'A', 'B'], ['A', 'B', 'body']],
                [['C', 'D'], ['A', 'B', 'C', 'D']],
                [['child-release'], ['C', 'D', 'child-release']],
                [[], ['D', 'child-release']],
                [[], ['D']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'body': 'b'},
                {'A': 'a'},
                {'C': 'c'},
                {'child-release': 'r'},
                {'D': 'd'},
                {'parent-release': 'p'},
            ],
            'result': 'ok',
        },
        'normal': {
            'steps': [
                [['A', 'discarded', 'B'], ['A', 'B', 'discarded']],
                [[], ['A', 'discarded']],
                [[], ['discarded']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'B': 'b'},
                {'A': 'a'},
                {'discarded': 'd'},
                {'parent-release': 'p'},
            ],
            'result': 'ok',
        },
        'failure': {
            'steps': [
                [['body', 'A', 'B'], ['A', 'B', 'body']],
                [['child-release'], ['child-release']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [{'A': 'a'}, {'child-release': 'r'}, {'parent-release': 'p'}],
            'result': 'ok',
        },
        'cancel': {
            'steps': [
                [['control', 'body', 'A', 'B'], ['A', 'B', 'body', 'control']],
                [
                    ['repeat', 'B-release', 'A-release'],
                    ['A-release', 'B-release', 'repeat'],
                ],
                [[], ['A-release', 'B-release']],
                [[], ['B-release']],
                [['owner-release'], ['owner-release']],
            ],
            'batches': [
                {'control': 'c'},
                {'repeat': 'r'},
                {'A-release': 'a'},
                {'B-release': 'b'},
                {'owner-release': 'o'},
            ],
            'result': 'ok',
        },
        'host-cancel': {
            'steps': [
                [['body', 'A', 'B'], ['A', 'B', 'body']],
                [['B-release', 'A-release'], ['A-release', 'B-release']],
                [[], ['B-release']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'$cancel': 'FIRST'},
                {'$cancel': 'REPEAT', 'A-release': 'a'},
                {'B-release': 'b'},
                {'parent-release': 'p'},
            ],
            'cancelled': 'FIRST',
        },
        'root-shutdown': {
            'steps': [
                [['control', 'body'], ['body', 'control']],
                [['child-release'], ['child-release']],
            ],
            'batches': [{'control': 'c'}, {'child-release': 'r'}],
            'result': 'ok',
        },
        'transferred-body-obligations': {
            'steps': [
                [['A', 'B'], ['A', 'B']],
                [[], ['B']],
                [['child-release'], ['child-release']],
                [['parent-release'], ['parent-release']],
            ],
            'batches': [
                {'A': 'a'},
                {'B': 'b'},
                {'child-release': 'r'},
                {'parent-release': 'p'},
            ],
            'result': 'later-error',
        },
    }
    programs['body-joined-child'] = programs['body-gather']
    programs['root-error'] = programs['root-shutdown']
    programs['failure-host'] = {
        'steps': [
            [['body', 'A', 'B'], ['A', 'B', 'body']],
            [['child-release'], ['child-release']],
            [['parent-release'], ['parent-release']],
        ],
        'batches': [
            {'A': 'a'},
            {'$cancel': 'FIRST', 'child-release': 'r'},
            {'parent-release': 'p'},
        ],
        'cancelled': 'FIRST',
    }
    programs.update(
        {
            'retained-AB': {
                'steps': [
                    [['A', 'B'], ['A', 'B']],
                    [['A-release', 'B-release'], ['A-release', 'B-release']],
                    [[], ['B-release']],
                    [['parent-release'], ['parent-release']],
                ],
                'batches': [{'A': 'a', 'B': 'b'}, {'A-release': 'r'}, {'B-release': 'r'}, {'parent-release': 'p'}],
                'result': 'ok',
            },
            'retained-BA': {
                'steps': [
                    [['A', 'B'], ['A', 'B']],
                    [['A-release', 'B-release'], ['A-release', 'B-release']],
                    [[], ['A-release']],
                    [['parent-release'], ['parent-release']],
                ],
                'batches': [{'A': 'a', 'B': 'b'}, {'B-release': 'r'}, {'A-release': 'r'}, {'parent-release': 'p'}],
                'result': 'ok',
            },
            'outer-failure-inner-exit': {
                'steps': [[['A', 'B'], ['A', 'B']], [['B-release'], ['B-release']]],
                'batches': [{'A': 'a'}, {'B-release': 'r'}],
                'result': 'ok',
            },
            'retained-cyclic': {
                'steps': [
                    [['A', 'B'], ['A', 'B']],
                    [['A-release', 'B-release'], ['A-release', 'B-release']],
                    [[], ['B-release']],
                    [['parent-release'], ['parent-release']],
                ],
                'batches': [{'A': 'a', 'B': 'b'}, {'A-release': 'r'}, {'B-release': 'r'}, {'parent-release': 'p'}],
                'result': 'ok',
            },
        }
    )
    for name in programs:
        (args.output / 'cases' / (name + '.py')).write_bytes((ROOT / 'cases' / (name + '.py')).read_bytes())
    gate.ROOT = args.output.resolve()
    report = {
        'results': {},
        'association_sha256': gate.digest(args.association),
        'disk_before': before,
    }
    for backend in ('native', 'wasm'):
        for name, case in programs.items():
            result = gate.run_case(name, case, backend, args.output / (backend + '-' + name))
            if result['status'] == 'PASS' and 'cancelled' in case:
                evidence = json.loads((args.output / (backend + '-' + name) / 'evidence.json').read_text())
                try:
                    check_evidence(evidence, name, gate)
                except (ValueError, KeyError, TypeError) as error:
                    result = {'status': 'FAIL', 'reason': str(error)}
            report['results'][backend + '/' + name] = result
            print(backend, name, result, flush=True)
    gate.ROOT = ROOT.parent / 'native_async'
    gate.association(args.association)
    gate.frozen()
    report['disk_after'] = gate.budget()
    report['status'] = 'PASS' if all(r['status'] == 'PASS' for r in report['results'].values()) else 'FAIL'
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return int(report['status'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
