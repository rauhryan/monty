"""Extend the accepted guarded build receipt with the queue gate and tests."""
import argparse
import json
import subprocess
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'concurrent_io'))
from bind import REPO, digest


def main():
    """Hash before building; reject any source drift during artifact production."""
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output = args.output.resolve()
    files = {str(p.relative_to(REPO)): digest(p) for p in ROOT.iterdir() if p.is_file()}
    for name in (
        'crates/monty/tests/unit/guest_queue.rs',
        'crates/monty/tests/unit/queue_repl.rs',
        'crates/monty/tests/fixtures/dump_v27.bin',
        'docs/limitations/asyncio.md',
    ):
        files[name] = digest(REPO / name)
    subprocess.run([sys.executable, str(ROOT.parent / 'concurrent_io/bind.py'), '--output', str(args.output)], check=True)
    assert files == {name: digest(REPO / name) for name in files}
    association = json.loads((args.output / 'concurrent-association.json').read_text())
    association['files'].update(files)
    # Preserve reviewable source independently of mutable working-tree paths.
    patch = args.output / 'source.patch'
    patch.write_bytes(subprocess.check_output(['git', 'diff', '--binary', 'HEAD'], cwd=REPO))
    added = subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard'], cwd=REPO, text=True).splitlines()
    for name in added:
        target = args.output / 'added' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(REPO / name, target)
    association['baseline'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    association['source_archive'] = {str(p.relative_to(REPO)): digest(p) for p in [patch, *(args.output / 'added').rglob('*')] if p.is_file()}
    (args.output / 'queues-association.json').write_text(json.dumps(association, indent=2) + '\n')


if __name__ == '__main__':
    main()
