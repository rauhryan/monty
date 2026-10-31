"""Real native/Wasm baseline adapter; JSON request on stdin, one response on stdout."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
BASE = REPO / 'playground/coroutine-baseline'


def source(mode, directory=None):
    """Load only an explicit source directory, never import mutable coordinator modules."""
    directory = Path(directory) if directory else ROOT / 'reference'
    body = '\n'.join((directory / name).read_text() for name in ('bindings.py', 'kernel.py', 'coordinator.py'))
    body += '\nimport json\n' + (ROOT / 'workloads.py').read_text() + '\n' + (ROOT / 'bridge.py').read_text()
    return body + f'\nstate = new_driver({mode!r}, False)\n'


def invoke(command, args, evidence):
    """A fresh producer/consumer/disposer process; reject all diagnostics and failures."""
    result = subprocess.run([*command, *map(str, args)], capture_output=True, text=True, timeout=30)
    evidence.append(
        {
            'argv': [*command, *map(str, args)],
            'exit': result.returncode,
            'stdout': result.stdout,
            'stderr': result.stderr,
        }
    )
    if result.returncode or result.stderr:
        raise ValueError(evidence[-1])
    return json.loads(result.stdout)


def run(request):
    """Execute one complete authored case with explicit physical restore evidence."""
    case = request['case']
    directory = Path(request['output'])
    directory.mkdir(parents=True, exist_ok=False)
    command = (
        [str(BASE / 'target/debug/coroutine-baseline')]
        if request['backend'] == 'native'
        else ['node', str(BASE / 'wasm.mjs')]
    )
    body = source(case['mode'], request.get('source_directory'))
    body = body.replace('\nstate = new_driver(', '\ndef adapt(body):\n    return body\n\nstate = new_driver(')
    evidence = []
    views = []
    if request['execution'] == 'uninterrupted':
        guest = directory / 'guest.py'
        guest.write_text(
            body
            + '\nviews = [json.loads(json.dumps(step([])))]\n'
            + f'for messages in {case["batches"]!r}:\n    views.append(json.loads(json.dumps(step(messages))))\n'
            + 'json.dumps(views)\n'
        )
        actual = invoke(command, ['start', guest, directory / 'unused.bin'], evidence)
        if set(actual) != {'complete'}:
            raise ValueError(actual)
        views = actual['complete']
    else:
        guest = directory / 'guest.py'
        guest.write_text(
            body
            + '\nobservation = step([])\nwhile observation["result"] is None:\n'
            + '    messages = json.loads(exchange(json.dumps(observation)))\n'
            + '    observation = step(messages)\njson.dumps(observation)\n'
        )
        snapshot = directory / 'cut-0.bin'
        actual = invoke(command, ['start', guest, snapshot], evidence)
        if set(actual) != {'effect'}:
            raise ValueError(actual)
        views.append(actual['effect'])
        for i, batch in enumerate(case['batches']):
            before = snapshot.read_bytes()
            evidence.append({'snapshot': str(snapshot), 'sha256': hashlib.sha256(before).hexdigest()})
            if invoke(command, ['dispose', snapshot], evidence) != {'disposed': True}:
                raise ValueError('dispose failed')
            if case['mode'] == 'nested' and i == 0:
                divergent = directory / 'divergent.bin'
                altered = [dict(batch[0], value='branch')]
                branch_view = invoke(command, ['resume', snapshot, json.dumps(altered), divergent], evidence)
                if branch_view['effect']['trace'] != ['branch']:
                    raise ValueError('divergent branch did not consume altered input')
                if invoke(command, ['dispose', divergent], evidence) != {'disposed': True}:
                    raise ValueError('divergent disposal failed')
            target = directory / f'cut-{i + 1}.bin'
            actual = invoke(command, ['resume', snapshot, json.dumps(batch), target], evidence)
            key = 'complete' if i == len(case['batches']) - 1 else 'effect'
            if set(actual) != {key}:
                raise ValueError(actual)
            # Same bytes, independent consumer. Disposal must not alter recovery.
            branch = directory / f'branch-{i}.bin'
            repeated = invoke(command, ['resume', snapshot, json.dumps(batch), branch], evidence)
            if repeated != actual or snapshot.read_bytes() != before:
                raise ValueError('branch recovery mismatch')
            if key == 'effect' and invoke(command, ['dispose', branch], evidence) != {'disposed': True}:
                raise ValueError('branch disposal failed')
            views.append(actual[key])
            snapshot = target
    return {
        'views': views,
        'evidence': evidence,
        'loaded_source_sha256': hashlib.sha256(guest.read_bytes()).hexdigest(),
    }


if __name__ == '__main__':
    print(json.dumps(run(json.load(sys.stdin))))
