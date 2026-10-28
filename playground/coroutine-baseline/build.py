"""Budgeted build harness: one reusable target, serialized Cargo, bounded disk usage."""

import argparse
import fcntl
import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
TARGET = ROOT / 'target'
REVIEW_TARGET = REPO / 'playground/coroutine-review-target'
GIB = 1024**3


def usage():
    """Measure both agreed caches and the filesystem holding the candidate."""
    sizes = {}
    for target in (TARGET, REVIEW_TARGET):
        size = 0
        for path in target.rglob('*'):
            try:
                if path.is_file():
                    size += path.stat().st_blocks * 512
            except FileNotFoundError:
                pass  # Cargo may rename/remove a temporary file while being measured.
        sizes[str(target)] = size
    return {'free': shutil.disk_usage(REPO).free, 'targets': sizes, 'combined': sum(sizes.values())}


def require_budget(measured):
    """Refuse another build below 10 GiB free or above the combined 8 GiB cache cap."""
    if measured['free'] < 10 * GIB or measured['combined'] > 8 * GIB:
        raise RuntimeError(f'build budget exceeded: {measured}')


def active_processes():
    """Find competing Cargo/rustc jobs and running artifacts before cleanup."""
    active = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            comm = (entry / 'comm').read_text().strip()
            command = (entry / 'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
            executable = str((entry / 'exe').resolve())
        except (OSError, ProcessLookupError):
            continue
        if comm in ('cargo', 'cargo-clippy', 'rustc', 'clippy-driver') or any(
            str(target) in executable or str(target) in command for target in (TARGET, REVIEW_TARGET)
        ):
            active.append({'pid': int(entry.name), 'command': command})
    return active


def prune():
    """Delete only inactive candidate intermediates; preserve binaries, dumps and all evidence."""
    if active_processes():
        raise RuntimeError('refusing cleanup while build or target process is active')
    before = usage()
    removed = []
    # Reviewer output is measured, never removed by the candidate's cleanup.
    for profile in (TARGET / 'debug', TARGET / 'release', TARGET / 'wasm32-wasip1/release'):
        for name in ('deps', 'build', '.fingerprint', 'incremental', 'examples'):
            directory = profile / name
            if directory.is_dir():
                if directory.is_symlink() or not directory.resolve().is_relative_to(TARGET.resolve()):
                    raise RuntimeError('refusing cleanup through a symlink')
                shutil.rmtree(directory)
                removed.append(str(directory))
    after = usage()
    return {'before': before, 'after': after, 'removed': removed, 'freed_bytes': before['combined'] - after['combined']}


def main():
    """All future campaign builds enter here; execution checks share the same target."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        'action',
        choices=[
            'policy7-proto-tests',
            'policy7-subprocess-tests',
            'policy7-runtime-regressions',
            'native7-ownership-tests',
            'repair10-fatal-tests',
            'repair10-wasm',
            'typescript',
            'generate-proto',
            'proto-tests',
            'protocol-subprocess-tests',
            'proto-clippy',
            'protocol-subprocess-clippy',
            'async-compat-tests',
            'check',
            'clean',
            'native',
            'wasm',
            'component',
            'lint-rs',
            'lint-py',
            'native-clippy',
            'format-rs',
            'native-check',
            'wasm-check',
            'generator-compile-tests',
            'generator-metadata-tests',
            'generator-runtime-tests',
            'generator-runtime-clippy',
            'ownership-unit-tests',
            'dump-compat-tests',
        ],
    )
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('timeout must be positive')
    env = os.environ | {
        'PATH': '/home/paseo/.cargo/bin:' + os.environ['PATH'],
        'CARGO_TARGET_DIR': str(TARGET),
        'CARGO_INCREMENTAL': '0',
        'CARGO_PROFILE_DEV_DEBUG': '0',
        'CARGO_PROFILE_TEST_DEBUG': '0',
        'CARGO_BUILD_JOBS': '2',
    }
    commands = {
        'policy7-proto-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty-proto',
            '--features',
            'worker',
            '--test',
            'differential',
            '--test',
            'dispatch',
            '--test',
            'protocol_compatibility',
            '--',
            '--skip',
            'protocol_native_cancel_dispatch_preserves_presence',
        ],
        'policy7-subprocess-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty-runtime',
            '--no-default-features',
            '--test',
            'subprocess',
            'protocol_',
            '--',
            '--skip',
            'protocol_native_cancel_subprocess_preserves_presence',
        ],
        'policy7-runtime-regressions': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks,ref-count-return',
            '--test',
            'generator_runtime',
            '--',
            '--skip',
            'native2_cancelled_cleanup_live_ownership',
            '--skip',
            'native4_cleanup_exit_pending_owner_census',
            '--skip',
            'native5_dispatch_error_owner_census',
            '--skip',
            'native6_active_dispatch_owner_census',
        ],
        'native7-ownership-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks,ref-count-return',
            '--test',
            'generator_runtime',
            'native7_',
            '--',
            '--nocapture',
        ],
        'typescript': [
            'node',
            'crates/monty-js/node_modules/typescript/bin/tsc',
            '-p',
            'crates/monty-js/tsconfig.json',
        ],
        'generate-proto': ['make', 'generate-proto'],
        'proto-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty-proto',
            '--features',
            'worker',
            '--test',
            'differential',
            '--test',
            'dispatch',
            '--test',
            'protocol_compatibility',
        ],
        'protocol-subprocess-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty-runtime',
            '--no-default-features',
            '--test',
            'subprocess',
            'protocol_',
        ],
        'proto-clippy': [
            'cargo',
            'clippy',
            '--locked',
            '-p',
            'monty-proto',
            '--features',
            'worker',
            '--test',
            'differential',
            '--test',
            'dispatch',
            '--test',
            'protocol_compatibility',
            '--',
            '-D',
            'warnings',
            '-A',
            'clippy::unnecessary_box_returns',
        ],
        'protocol-subprocess-clippy': [
            'cargo',
            'clippy',
            '--locked',
            '-p',
            'monty-runtime',
            '--no-default-features',
            '--test',
            'subprocess',
            '--',
            '-D',
            'warnings',
            '-A',
            'clippy::unnecessary_box_returns',
        ],
        'async-compat-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks',
            '--test',
            'asyncio',
        ],
        'ownership-unit-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks,ref-count-return',
            '--lib',
            'ownership_',
        ],
        'generator-runtime-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks,ref-count-return',
            '--test',
            'generator_runtime',
        ],
        'generator-runtime-clippy': [
            'cargo',
            'clippy',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks,ref-count-return',
            '--test',
            'generator_runtime',
            '--',
            '-D',
            'warnings',
            '-A',
            'clippy::unnecessary_box_returns',
        ],
        'dump-compat-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks',
            '--test',
            'dump_compat',
        ],
        'generator-metadata-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks',
            '--lib',
            'serialized_components_match_dump_version',
        ],
        'generator-compile-tests': [
            'cargo',
            'test',
            '--locked',
            '-p',
            'monty',
            '--features',
            'memory-model-checks',
            '--test',
            'generator_compile',
        ],
        'native-check': ['cargo', 'check', '--locked', '--manifest-path', str(ROOT / 'native/Cargo.toml')],
        'wasm-check': ['cargo', 'check', '--locked', '-p', 'monty', '--target', 'wasm32-wasip1'],
        'native': ['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'native/Cargo.toml')],
        'wasm': ['cargo', 'build', '--locked', '-p', 'monty-wasm-runtime', '--target', 'wasm32-wasip1', '--release'],
        'component': ['node', 'crates/monty-js/scripts/build-component.mjs'],
        'lint-rs': ['make', 'lint-rs'],
        'lint-py': ['make', 'lint-py'],
        'format-rs': ['make', 'format-rs'],
        'native-clippy': [
            'cargo',
            'clippy',
            '--locked',
            '--manifest-path',
            str(ROOT / 'native/Cargo.toml'),
            '--',
            '-D',
            'warnings',
            '-A',
            'clippy::unnecessary_box_returns',
        ],
    }
    commands['repair10-fatal-tests'] = [
        'cargo',
        'test',
        '--locked',
        '-p',
        'monty',
        '--features',
        'memory-model-checks,ref-count-return,cancellation-test-hooks',
        '--test',
        'generator_runtime',
        'native7_repair10',
        '--',
        '--nocapture',
    ]
    commands['repair10-wasm'] = commands['wasm'] + ['--features', 'cancellation-test-hooks']
    evidence = {
        'action': args.action,
        'limits': {'minimum_free': 10 * GIB, 'cache_cap': 8 * GIB, 'jobs': 2},
    }
    code = 0
    with (ROOT / 'build.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.report.exists() or args.report.with_suffix('.log').exists():
            raise RuntimeError('refusing to overwrite build evidence')
        evidence['before'] = usage()
        if args.action == 'clean':
            evidence['cleanup'] = prune()
        elif args.action == 'check':
            require_budget(evidence['before'])
            evidence['active'] = active_processes()
        else:
            if active_processes():
                raise RuntimeError(f'competing build/target process: {active_processes()}')
            evidence['command'] = commands[args.action]
            with args.report.with_suffix('.log').open('x') as log:
                evidence['before'] = usage()
                require_budget(evidence['before'])
                process = subprocess.Popen(
                    commands[args.action],
                    cwd=REPO,
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
                started = time.monotonic()
                try:
                    while process.poll() is None:
                        require_budget(usage())
                        if time.monotonic() - started > args.timeout:
                            raise TimeoutError('bounded build timeout')
                        time.sleep(1)
                    code = process.returncode
                except BaseException as error:
                    evidence['stopped'] = str(error)
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    code = 1
            evidence['exit'] = code
        evidence['after'] = usage()
        try:
            require_budget(evidence['after'])
        except RuntimeError as error:
            evidence['stopped'] = str(error)
            code = 1
        evidence['exit'] = code
        evidence['status'] = 'PASS' if code == 0 else 'FAIL'
        with args.report.open('x') as output:
            output.write(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps(evidence))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
