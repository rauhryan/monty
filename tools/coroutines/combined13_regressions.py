"""Replay the unchanged eight R11-1 cases with the new source/library association."""

import argparse
import json
from pathlib import Path
from trace import digest, finite_dumps

import combined11 as c
import combined12 as r12
import scope9
from verify import BASELINE, REPO, artifacts, source_identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    receipt = json.loads((c.ROOT / 'combined13_artifacts.json').read_text())
    assert receipt['runtime_sha256'] == scope9.runtime_identity()
    assert receipt['source_sha256'] == source_identity()
    for path, sha256 in receipt['combined_inputs'].items():
        assert digest(Path(path)) == sha256
    scripts = (
        [r12.nested(stage, children) for children in (False, True) for stage in ('drain', 'queued-cleanup')]
        + [r12.original_error()]
        + r12.fixtures()
    )
    report = {'status': 'FAIL', 'cases': {}, 'association': receipt}
    with args.report.open('x') as output:
        try:
            for backend in ('native', 'wasm'):
                assert {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)} == receipt[backend][
                    'artifacts'
                ]
                command = (
                    [str(BASELINE / 'target/debug/coroutine-baseline')]
                    if backend == 'native'
                    else ['node', str(BASELINE / 'wasm.mjs')]
                )
                for script in scripts:
                    scope9.guest_source = lambda fixture=script.get('fixture', False): r12.guest_source(fixture)
                    path = args.report.parent / (args.report.stem + '-cuts') / backend / script['name']
                    report['cases'].setdefault(script['name'], {})[backend] = c.run(command, script, path)
                    print(backend, script['name'], 'PASS', flush=True)
            report['status'] = 'PASS'
        except (AssertionError, ValueError, OSError) as error:
            report['error'] = str(error)
        output.write(finite_dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
