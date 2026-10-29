"""Bounded CPython references for ordinary synchronous run wrappers."""

import ast
import asyncio
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def main():
    for name in (
        'root-wrapper',
        'root-wrapper-error',
        'root-deep',
        'root-deep-error',
        'root-cyclic-false',
        'root-cyclic-true',
        'root-sequential',
    ):
        calls = []

        async def exchange(label):
            calls.append(label)
            loop = asyncio.get_running_loop()
            future = loop.create_future()
            if not label.startswith('body'):
                loop.call_soon(future.set_result, 'reply')
            return await future

        path = ROOT / 'cases' / (name + '.py')
        module = ast.parse(path.read_text(), filename=str(path))
        last = module.body.pop()
        namespace = {'exchange': exchange}
        exec(compile(module, str(path), 'exec'), namespace)
        result = eval(compile(ast.Expression(last.value), str(path), 'eval'), namespace)
        assert json.loads(result) == ('caught' if name.endswith('error') else 'ok')
        expected = (
            ['control1', 'body1', 'release1', 'control2', 'body2', 'release2']
            if name == 'root-sequential'
            else ['control', 'body', 'child-release']
        )
        assert calls == expected, (name, calls)
        print(name, 'CPython PASS')
    print('Ordinary-gather cleanup barriers and host cancellation remain platform contracts.')


if __name__ == '__main__':
    main()
