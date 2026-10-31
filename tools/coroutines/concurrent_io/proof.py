"""Execute the supported concurrent slice, leaving the full fixed gate red."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from gate import ROOT, validate

REPO = ROOT.parents[2]


def invoke(command, args, evidence):
    """A fresh process/instance for each restored branch and disposal."""
    result = subprocess.run(
        [*command, *map(str, args)], capture_output=True, text=True, timeout=40
    )
    evidence.append(
        {
            'argv': result.args,
            'exit': result.returncode,
            'stdout': result.stdout,
            'stderr': result.stderr,
        }
    )
    assert result.returncode == 0 and not result.stderr, evidence[-1]
    return json.loads(result.stdout)


def main():
    """Check literal results plus ID-based reversed single-reply advancement."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--association', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    association = json.loads(args.association.read_text())
    for group in ('runtime', 'build_inputs', 'files', 'artifacts', 'receipts'):
        for name, digest in association[group].items():
            assert hashlib.sha256((REPO / name).read_bytes()).hexdigest() == digest, (
                name
            )
    expected = json.loads((ROOT / 'expected.json').read_text())
    results = {}
    for backend in ('native', 'wasm'):
        directory = args.output / backend
        directory.mkdir()
        command = (
            [str(REPO / 'playground/coroutine-baseline/target/debug/concurrent_io')]
            if backend == 'native'
            else ['node', str(ROOT / 'wasm.mjs')]
        )
        evidence = []
        snapshot = directory / 'cut-0.bin'
        first = invoke(command, ['start', ROOT / 'guest.py', snapshot], evidence)
        assert [call['label'] for call in first['calls']] == expected['1']['calls']
        ids = {call['label']: call['id'] for call in first['calls']}
        assert len(set(ids.values())) == 2 and sorted(ids.values()) == first['pending']
        batches = [
            [{'id': ids['B'], 'value': 'beta'}],
            [{'id': ids['A'], 'value': 'alpha'}],
        ]
        restored = [first]
        for index, batch in enumerate(batches):
            original = snapshot.read_bytes()
            evidence.append(
                {
                    'snapshot': str(snapshot),
                    'sha256': hashlib.sha256(original).hexdigest(),
                }
            )
            assert invoke(command, ['dispose', snapshot], evidence) == {
                'disposed': True
            }
            target = directory / f'cut-{index + 1}.bin'
            actual = invoke(
                command, ['resume', snapshot, json.dumps(batch), target], evidence
            )
            branch = directory / f'branch-{index}.bin'
            repeated = invoke(
                command, ['resume', snapshot, json.dumps(batch), branch], evidence
            )
            assert actual == repeated and snapshot.read_bytes() == original
            if 'pending' in actual:
                assert actual == {'calls': [], 'pending': [ids['A']]}
                assert invoke(command, ['dispose', branch], evidence) == {
                    'disposed': True
                }
            restored.append(actual)
            snapshot = target
        batches_file = directory / 'batches.json'
        batches_file.write_text(json.dumps(batches))
        continuous = invoke(command, ['run', ROOT / 'guest.py', batches_file], evidence)
        assert continuous == restored
        observation = {
            'calls': [c['label'] for c in first['calls']],
            **restored[-1]['complete'],
        }
        assert observation == expected['1']
        results[backend] = {'views': restored, 'evidence': evidence}
    assert results['native']['views'] == results['wasm']['views']
    # Requirement 8 covers ALL fixed cases: this round proves only requirement 1's recovery.
    observations = {'1': expected['1']}
    (args.output / 'observations.json').write_text(
        json.dumps(observations, indent=2) + '\n'
    )
    report = {
        'gate': validate(observations),
        'recovery_slice': 'PASS: requirement 1 on native/Wasm, uninterrupted and fresh restored/disposed branches',
        'association': str(args.association),
        'association_sha256': hashlib.sha256(args.association.read_bytes()).hexdigest(),
        'results': results,
    }
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['gate'], indent=2))


if __name__ == '__main__':
    main()
