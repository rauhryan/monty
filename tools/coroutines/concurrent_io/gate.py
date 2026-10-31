#!/usr/bin/env python3
"""Fixed-scope acceptance: absent capabilities are failures, never skipped passes."""

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def validate(observations):
    """Compare literal observations frozen before implementing the candidate."""
    expected_bytes = (ROOT / 'expected.json').read_bytes()
    assert (
        hashlib.sha256(expected_bytes).hexdigest()
        == (ROOT / 'expected.sha256').read_text().split()[0]
    )
    expected = json.loads(expected_bytes)
    return {
        key: 'PASS'
        if observations.get(key) == value
        else 'FAIL: unsupported or unequal observations'
        for key, value in expected.items()
    }


def main():
    """Read evidence produced by a candidate, with no generated oracle."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--observations', type=Path)
    args = parser.parse_args()
    observations = (
        json.loads(args.observations.read_text()) if args.observations else {}
    )
    expected = json.loads((ROOT / 'expected.json').read_text())
    # Comparator negative controls: wrong routing, early cleanup, lost sticky reason.
    for key, field, bad in (
        ('1', 'joined', [['A', 'beta'], ['B', 'alpha']]),
        (
            '4',
            'trace',
            ['parent-cleanup', 'cancel-A', 'cancel-B', 'drain-A', 'drain-B'],
        ),
        ('6', 'reason', 'SECOND'),
    ):
        altered = expected | {key: expected[key] | {field: bad}}
        assert validate(altered)[key].startswith('FAIL')
    result = validate(observations)
    print(json.dumps(result, indent=2))
    return int(any(value != 'PASS' for value in result.values()))


if __name__ == '__main__':
    raise SystemExit(main())
