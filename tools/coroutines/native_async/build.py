"""Build and associate the clean-entry adapters through the existing disk guard."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'native1'))
from bind import REPO, digest, inputs  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    source = {str(p.relative_to(REPO)): digest(p) for p in ROOT.rglob('*') if p.is_file() and '__pycache__' not in str(p)}
    before = inputs()
    subprocess.run([sys.executable, str(ROOT.parent / 'native1/bind.py'), '--output', str(output)], check=True)
    assert before == inputs(), 'source changed during build'
    assert all(digest(REPO / p) == h for p, h in source.items())
    association = json.loads((output / 'build-association.json').read_text())
    association['files'].update(source)
    binary = REPO / 'playground/coroutine-baseline/target/debug/native_async'
    association['artifacts'][str(binary.relative_to(REPO))] = digest(binary)
    (output / 'native-async-association.json').write_text(json.dumps(association, indent=2) + '\n')
    print(output / 'native-async-association.json')


if __name__ == '__main__':
    main()
