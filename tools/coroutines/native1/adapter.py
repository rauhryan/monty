"""Partial candidate: execute pinned nested and host workloads with native-owned awaits."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def invoke(command, args, evidence):
    """Keep raw process evidence and refuse transport or interpreter failures."""
    result = subprocess.run([*command, *map(str, args)], capture_output=True, text=True, timeout=40)
    evidence.append({'argv': result.args, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
    if result.returncode or result.stderr:
        raise ValueError(evidence[-1])
    decoded = json.loads(result.stdout)
    decoded.pop('raw_envelope', None)
    decoded.pop('raw_pending_ids', None)
    return decoded


def source(mode):
    """Compile authored async-with unchanged; explicit Rust entry replaces a trampoline."""
    if mode not in ('nested', 'host'):
        raise NotImplementedError('native1 unsupported workload: ' + mode)
    return (
        (ROOT / 'scope.py').read_text()
        + '\n'
        + (ROOT.parent / 'conformance/workloads.py').read_text()
        + f'\nresult = parent_body(None, None, {mode!r})._native1()\n'
        + "json.dumps({'events': PUBLIC, 'trace': TRACE, 'result': ['value', result, None, None, [], []]})\n"
    )


def execute(body, batches, backend, execution, directory, divergent=False):
    """Run real pending futures; restore/dispose in separate OS processes at every cut."""
    directory.mkdir(parents=True, exist_ok=False)
    guest = directory / 'guest.py'
    guest.write_text(body)
    command = (
        [str(REPO / 'playground/coroutine-baseline/target/debug/native1')]
        if backend == 'native'
        else ['node', str(ROOT / 'wasm.mjs')]
    )
    evidence = []
    if execution == 'uninterrupted':
        script = directory / 'batches.json'
        script.write_text(json.dumps(batches))
        views = invoke(command, ['run', guest, script], evidence)['views']
    else:
        snapshot = directory / 'cut-0.bin'
        actual = invoke(command, ['start', guest, snapshot], evidence)
        views = [actual['effect']]
        for i, batch in enumerate(batches):
            before = snapshot.read_bytes()
            evidence.append({'snapshot': str(snapshot), 'sha256': hashlib.sha256(before).hexdigest()})
            assert invoke(command, ['dispose', snapshot], evidence) == {'disposed': True}
            if divergent and i == 0:
                branch = directory / 'divergent.bin'
                altered = [dict(batch[0], value='branch')]
                different = invoke(command, ['resume', snapshot, json.dumps(altered), branch], evidence)
                assert different['effect']['trace'] == ['branch']
                assert invoke(command, ['dispose', branch], evidence) == {'disposed': True}
            target = directory / f'cut-{i + 1}.bin'
            actual = invoke(command, ['resume', snapshot, json.dumps(batch), target], evidence)
            branch = directory / f'branch-{i}.bin'
            repeated = invoke(command, ['resume', snapshot, json.dumps(batch), branch], evidence)
            assert actual == repeated and snapshot.read_bytes() == before
            if actual == {'waiting': True}:
                actual = {'effect': views[-1]}
            key = 'complete' if i == len(batches) - 1 else 'effect'
            assert set(actual) == {key}
            if key == 'effect':
                assert invoke(command, ['dispose', branch], evidence) == {'disposed': True}
            views.append(actual[key])
            snapshot = target
    return {
        'views': views,
        'evidence': evidence,
        'loaded_source_sha256': hashlib.sha256(guest.read_bytes()).hexdigest(),
    }


def run(request):
    """No fallback: unsupported scopes/queues fail the mandatory gate."""
    mode = request['case']['mode']
    batches = request['case']['batches']
    if mode == 'host':
        batches = [
            {
                'native_cancel': next(m['reason'] for m in batch if m['kind'] == 'cancel'),
                'native_results': [{'reply': [m]} for m in batch if m['kind'] == 'reply'],
            }
            for batch in batches
        ]
    result = execute(
        source(mode),
        batches,
        request['backend'],
        request['execution'],
        Path(request['output']),
        divergent=mode == 'nested',
    )
    if mode == 'host':
        terminal = result['views'][-1]
        assert terminal['status'] == 'cancelled' and terminal['error'] is None
        view = json.loads(terminal['value'])
        view['result'][0] = 'cancelled'
        view['result'][2:4] = [terminal['reason'], terminal['reason']]
        result['views'][-1] = view
    return result


if __name__ == '__main__':
    print(json.dumps(run(json.load(sys.stdin))))
