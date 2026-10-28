"""Rebuild authenticated historical source using the existing shared-lock build guard."""

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('guard', ROOT / 'tools/coroutines/coordinator1/guarded_checks.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
# Keep TARGET, lock, environment, budgets and the wasm command unchanged. Only
# Cargo's working directory changes to the frozen historical source checkout.
guard.build.REPO = ROOT / 'playground/coroutine-implementation/catchable-cancellation-compatibility11/historical-source'
if sys.argv[1] == 'component':
    guard.build.REPO = ROOT
    original_popen = guard.build.subprocess.Popen

    def component_popen(command, **kwargs):
        assert command == ['node', 'crates/monty-js/scripts/build-component.mjs']
        return original_popen(['node', 'tools/coroutines/catchable_cancellation11/component.mjs'], **kwargs)

    guard.build.subprocess.Popen = component_popen
if __name__ == '__main__':
    raise SystemExit(guard.build.main())
