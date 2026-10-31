"""Pinned standard overlap, reusing the frozen upstream checkpoint reference."""

import asyncio
import importlib.metadata
import importlib.util
import json
import platform
from pathlib import Path

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('reference', ROOT.parent / 'anyio_feasibility/reference.py')
reference = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reference)


def main():
    contract = json.loads((ROOT / 'contract.json').read_text())
    assert platform.python_version() == contract['reference']['python']
    assert importlib.metadata.version('anyio') == contract['reference']['anyio']
    reference.frozen()
    reference.ROOT = ROOT
    cases = contract['scope_targets'] | {n: c for n, c in contract['additional'].items() if not c.get('platform')}
    print(
        json.dumps(
            {
                'reference': contract['reference'],
                'results': {n: asyncio.run(reference.execute(n, c)) for n, c in cases.items()},
            },
            indent=2,
        )
    )


if __name__ == '__main__':
    main()
