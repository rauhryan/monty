"""CPython semantics only: live generators are never presented as snapshots."""

import ast
import json
import platform
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def reference(case):
    """Evaluate the same fixture with deterministic accepted host inputs."""
    trace = []
    replies = iter(case['replies'])

    def exchange(payload):
        """Record the effect before applying precisely one accepted reply."""
        trace.append({'effect': json.loads(payload)})
        reply = next(replies)
        trace.append({'reply': reply})
        if 'raise' in reply:
            raise ValueError(reply['raise'])
        return json.dumps(reply)

    path = ROOT / 'cases' / case['fixture']
    tree = ast.parse(path.read_text(), filename=case['fixture'])
    terminal = ast.Expression(tree.body.pop().value)
    namespace = {'exchange': exchange}
    exec(compile(tree, case['fixture'], 'exec'), namespace)
    trace.append({'complete': json.loads(eval(compile(terminal, case['fixture'], 'eval'), namespace))})
    assert next(replies, None) is None
    return trace


if __name__ == '__main__':
    assert platform.python_implementation() == 'CPython'
    assert sys.version_info[:2] == (3, 14)
    cases = json.loads((ROOT / 'cases.json').read_text())
    print(json.dumps({name: reference(case) for name, case in cases.items()}, sort_keys=True))
