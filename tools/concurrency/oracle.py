"""CPython adapter for ordinary async fixtures; host cancellation has no oracle here."""

import asyncio
import json
import sys
from pathlib import Path


def main():
    """Execute the same ordinary Python definitions with a CPython host-call stand-in."""
    if sys.version_info[:2] != (3, 14):
        raise RuntimeError('the oracle requires CPython 3.14')
    source_path = Path(sys.argv[1])
    labels = []

    async def exchange(label):
        labels.append(label)
        await asyncio.sleep(0)
        return 'ok'

    source = source_path.read_text()
    definitions = source.removesuffix('asyncio.run(main())\n')
    if definitions == source:
        raise ValueError('missing ordinary asyncio entry')
    namespace = {'exchange': exchange}
    exec(compile(definitions, str(source_path), 'exec'), namespace)
    result = asyncio.run(namespace['main']())
    print(json.dumps({'result': json.loads(result), 'labels': labels}))


if __name__ == '__main__':
    main()
