"""Serialized checks with the established shared lock, target and disk budgets.

Only outputs live under playground; this module loads no code from there.
Additional shared caches are supplied as an os.pathsep-separated environment value.
"""

import contextlib
import fcntl
import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
STATE = REPO / 'playground/coroutine-baseline'
TARGET = STATE / 'target'
GIB = 1024**3


def cache_roots():
    roots = [TARGET, REPO / 'target', REPO / 'playground/coroutine-review-target']
    roots.extend(
        Path(p).resolve()
        for p in os.environ.get('MONTY_CHECK_EXTRA_CACHES', '').split(os.pathsep)
        if p
    )
    return sorted(set(p.resolve() for p in roots))


def usage():
    sizes = {}
    for root in cache_roots():
        total = 0
        for path in root.rglob('*'):
            try:
                if path.is_file():
                    total += path.stat().st_blocks * 512
            except FileNotFoundError:
                pass  # Cargo atomically replaces intermediates while being measured.
        sizes[str(root)] = total
    return {
        'free': shutil.disk_usage(REPO).free,
        'targets': sizes,
        'combined': sum(sizes.values()),
    }


def require_budget(measured):
    if measured['free'] < 10 * GIB or measured['combined'] > 8 * GIB:
        raise RuntimeError(f'build budget exceeded: {measured}')


def active_processes():
    active = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            comm = (entry / 'comm').read_text().strip()
            executable = (entry / 'exe').resolve()
        except (OSError, ProcessLookupError):
            continue
        if comm in (
            'cargo',
            'cargo-clippy',
            'rustc',
            'clippy-driver',
        ) or executable.is_relative_to(TARGET):
            active.append(
                {'pid': int(entry.name), 'executable': str(executable), 'name': comm}
            )
    return active


@contextlib.contextmanager
def session():
    """Share the legacy runner's nonblocking lock, including across verification."""
    STATE.mkdir(parents=True, exist_ok=True)
    with (STATE / 'build.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active = active_processes()
        if active:
            raise RuntimeError(f'competing build/target processes: {active}')
        require_budget(usage())
        yield
        require_budget(usage())


def environment():
    """Use the same tool lookup and Cargo settings for commands and provenance."""
    return os.environ | {
        'PATH': str(Path.home() / '.cargo/bin') + os.pathsep + os.environ['PATH'],
        'CARGO_TARGET_DIR': str(TARGET),
        'CARGO_INCREMENTAL': '0',
        'CARGO_PROFILE_DEV_DEBUG': '0',
        'CARGO_PROFILE_TEST_DEBUG': '0',
        'CARGO_BUILD_JOBS': '2',
    }


def execute(command, report, timeout=900):
    """Caller holds session(); never overwrite evidence or prune caches implicitly."""
    env = environment()
    evidence = {
        'command': list(map(str, command)),
        'before': usage(),
        'limits': {'minimum_free': 10 * GIB, 'cache_cap': 8 * GIB, 'jobs': 2},
    }
    require_budget(evidence['before'])
    if report.exists() or report.with_suffix('.log').exists():
        raise RuntimeError('refusing to overwrite evidence')
    with report.with_suffix('.log').open('x') as log:
        process = subprocess.Popen(
            command,
            cwd=REPO,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            start = time.monotonic()
            while process.poll() is None:
                require_budget(usage())
                if time.monotonic() - start > timeout:
                    raise TimeoutError('bounded build timeout')
                time.sleep(1)
            evidence['exit'] = process.returncode
        except BaseException as error:
            evidence['stopped'] = str(error)
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass  # The command may have exited during the resource check.
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            evidence['exit'] = 1
    evidence['after'] = usage()
    try:
        require_budget(evidence['after'])
    except RuntimeError as error:
        evidence['stopped'] = str(error)
        evidence['exit'] = 1
    report.write_text(json.dumps(evidence, indent=2) + '\n')
    if evidence['exit']:
        raise RuntimeError(f'command failed; see {report.with_suffix(".log")}')
