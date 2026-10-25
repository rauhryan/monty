"""Replay one existing generator-backed scope proof, with literal policy assertions.

Run with PYTHONPATH=tools/coroutines. This selects tasks explicitly: it is an
unwind/storage feasibility probe, NOT the proposed autonomous guest scheduler.
"""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from trace import digest, finite_dumps

import combined11 as combined
import scope9
from verify import BASELINE, REPO, artifacts


def main():
    """Reuse unchanged artifacts and old driver; never rewrite historical receipts."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    with args.report.open('x') as output:
        receipt = json.loads((REPO / 'tools/coroutines/combined13_artifacts.json').read_text())
        # The checkpoint newly tracked the ignored adapter manifest. The old
        # helper includes every tracked Cargo.toml, even outside runtime crates.
        names = subprocess.check_output(['git', 'ls-files'], cwd=REPO).decode().splitlines()
        runtime = {
            name: digest(REPO / name)
            for name in sorted(names)
            if (name.startswith('crates/') and ('/src/' in name or name.endswith('Cargo.toml')))
            or name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
        }
        normalized = hashlib.sha256(finite_dumps(runtime, sort_keys=True).encode()).hexdigest()
        assert normalized == receipt['runtime_sha256']
        script = combined.Script('hybrid1-unwind', 'catch')
        script.setup()
        script.cancel('FIRST')
        script.step()
        script.cancel('REPEAT')
        script.drain()
        view = script.view()
        assert view['terminal'] == {
            'kind': 'cancelled',
            'value': 'caught-parent-success',
            'cause': 'FIRST',
            'cancel': 'FIRST',
            'failures': [],
            'children': [],
        }
        assert [e[1]['payload'] for e in view['events'] if e[0] == 'pause'] == [
            ['acquire', 'a'],
            ['use', 'T1-H'],
            ['acquire', 'b'],
            ['use', 'T2-H'],
            ['another-name', 'T2-H'],
            ['arbitrary-child', 'T2-H'],
            ['another-name', 'T1-H'],
            ['arbitrary-child', 'T1-H'],
            ['parent-arbitrary', 'P'],
        ]
        scope9.guest_source = combined.guest_source
        report = {'runtime_sha256': receipt['runtime_sha256'], 'backends': {}}
        for backend in ('native', 'wasm'):
            hashes = {str(p.relative_to(REPO)): digest(p) for p in artifacts(backend)}
            assert hashes == receipt[backend]['artifacts']
            command = (
                [str(BASELINE / 'target/debug/coroutine-baseline')]
                if backend == 'native'
                else [
                    'node',
                    str(BASELINE / 'wasm.mjs'),
                ]
            )
            directory = args.report.parent / (args.report.stem + '-cuts') / backend
            report['backends'][backend] = {
                'artifacts': hashes,
                **scope9.run(command, script.script, directory, 30),
            }
            print(backend, 'PASS', flush=True)
        output.write(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
