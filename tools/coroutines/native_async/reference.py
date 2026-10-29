"""Check ordinary-Python final values against CPython; never generate expectations."""
import ast
import asyncio
import json

from verify import ROOT, check, equal, frozen


async def execute(name, case):
    pending = {}
    position = 0
    calls = []
    loop = asyncio.get_running_loop()

    def deliver():
        nonlocal position
        if position == len(case['batches']):
            return
        batch = case['batches'][position]
        if all(label in pending for label in batch):
            position += 1
            for label, value in batch.items():
                pending.pop(label).set_result(value)
            loop.call_soon(deliver)

    async def exchange(label):
        check(label not in pending, 'duplicate reference wait')
        calls.append(label)
        future = loop.create_future()
        pending[label] = future
        loop.call_soon(deliver)
        return await future

    source = ROOT / 'cases' / (name + '.py')
    module = ast.parse(source.read_text(), filename=str(source))
    check(ast.unparse(module.body[-1]) == 'asyncio.run(main())', 'unexpected public entry')
    # Only replace the top-level run with this test driver's existing event loop.
    module.body.pop()
    namespace = {'exchange': exchange}
    exec(compile(module, str(source), 'exec'), namespace)
    result = await asyncio.wait_for(namespace['main'](), timeout=5)
    check(equal(json.loads(result), case['result']), 'CPython final value differs: ' + name)
    check(calls == [label for step in case['steps'] for label in step[0]], 'CPython call order differs')
    check(position == len(case['batches']) and not pending, 'incomplete reference')


if __name__ == '__main__':
    for name, case in frozen().items():
        if 'cancelled' in case:
            print(name, 'PLATFORM CONTRACT: no CPython host-cancellation equivalent')
        else:
            asyncio.run(execute(name, case))
            print(name, 'CPython PASS')
