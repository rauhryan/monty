"""Independent malformed-observation probes; no VM or candidate-derived expectations."""

import copy
import importlib.util
from pathlib import Path

SPEC = importlib.util.spec_from_file_location('contract', Path(__file__).resolve().parents[1] / 'contract.py')
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def main():
    """Exercise schema holes and loading, separately from built-in self-checks."""
    document = contract.loads(contract.CASES.read_text())
    records = contract.expected_records(document)
    index = next(i for i, r in enumerate(records) if r['observed']['terminal']['interrupted_error'] is not None)
    root = (index, 'observed')
    terminal = (*root, 'terminal')
    diagnostic = (*terminal, 'interrupted_error')
    changes = [
        ((*diagnostic, 'message_truncated'), 0),
        ((*diagnostic, 'message_truncated'), 0.0),
        ((*diagnostic, 'message_truncated'), 1),
        ((*diagnostic, 'message_truncated'), 'false'),
        ((*diagnostic, 'message'), False),
        ((*diagnostic, 'message'), 0),
        ((*diagnostic, 'message'), None),
        ((*diagnostic, 'message'), ''),
        ((*diagnostic, 'message'), 'a' * 4097),
        ((*diagnostic, 'message'), 'é' * 2049),
        ((*diagnostic, 'message'), '\ud800'),
        ((*diagnostic, 'type'), 'InventedError'),
        ((*diagnostic, 'type'), []),
        ((*diagnostic, 'representation'), 'full'),
        ((*terminal, 'reason'), None),
        ((*terminal, 'error'), False),
        ((*terminal, 'status'), 'complete'),
        ((*terminal, 'value_export_error'), False),
        ((*terminal, 'value_export_error'), 'unknown'),
        (terminal, []),
        (root, []),
        ((index, 'case'), 'unknown'),
        ((index, 'backend'), 'other'),
        ((index, 'mode'), 'other'),
        ((index, 'case'), False),
        ((*root, 'effects'), {}),
        ((*root, 'effects', 0, 'trace'), {}),
        ((*root, 'effects', 0, 'phase'), None),
    ]
    bad = []
    for path, value in changes:
        altered = copy.deepcopy(records)
        parent = altered
        for key in path[:-1]:
            parent = parent[key]
        parent[path[-1]] = value
        bad.append(altered)
    for path in ((index,), root, terminal, diagnostic, (*root, 'effects', 0)):
        for action in ('missing', 'extra'):
            altered = copy.deepcopy(records)
            obj = altered
            for key in path:
                obj = obj[key]
            if action == 'missing':
                del obj[next(iter(obj))]
            else:
                obj['unexpected'] = None
            bad.append(altered)
    bad.extend([records[:-1], records + [records[0]], {}, None])
    altered = copy.deepcopy(records)
    altered[index]['observed']['effects'].append({'phase': 'purchase', 'trace': []})
    bad.append(altered)
    for number, altered in enumerate(bad):
        try:
            contract.check(document, altered)
        except ValueError:
            continue
        raise AssertionError(f'Accepted malformed record {number}')
    raw_bad = ['{"x":0,"x":1}', '{"a":{"message":null,"message":""}}', 'NaN', 'Infinity', '1e999', '"\\ud800"']
    for raw in raw_bad:
        try:
            contract.loads(raw)
        except ValueError:
            continue
        raise AssertionError(f'Accepted malformed JSON {raw}')
    for message in (None, '', 'a' * 4096, 'é' * 2048):
        contract.summary(
            {'type': 'TypeError', 'message': message, 'message_truncated': False, 'representation': 'metadata-only'}
        )
    assert not contract.exact(False, 0)
    assert not contract.exact(False, 0.0)
    assert not contract.exact(None, '')
    print(
        f'PASS {len(bad)} observation negatives, {len(raw_bad)} JSON negatives, four schema boundaries; NO VM execution'
    )


if __name__ == '__main__':
    main()
