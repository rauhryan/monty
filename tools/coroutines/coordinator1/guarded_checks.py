"""Use the existing build lock/guard, including unrelated workspace target caches."""

import importlib.util
from pathlib import Path

BASELINE = Path(__file__).resolve().parents[3] / 'playground/coroutine-baseline'
spec = importlib.util.spec_from_file_location('build_guard', BASELINE / 'build.py')
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
original_usage = build.usage
workspace = BASELINE.parents[3]


def usage():
    measured = original_usage()
    for target in (
        workspace / 'repos/sweatpants/experiments/workflow-kernel/teal-lua-checkpoint/target',
        workspace / 'repos/sweatpants/.artifacts/rune-checkpoint/target',
    ):
        measured['targets'][str(target)] = sum(p.stat().st_blocks * 512 for p in target.rglob('*') if p.is_file())
    measured['combined'] = sum(measured['targets'].values())
    return measured


build.usage = usage
if __name__ == '__main__':
    raise SystemExit(build.main())
