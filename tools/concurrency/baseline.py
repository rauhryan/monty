"""Capture six candidate baseline states and replay their exact bytes on both backends."""

import json
import shutil
import subprocess
from pathlib import Path

import guard
import replay

ROOT = Path(__file__).resolve().parent
FORMAT = 40


def census(state):
    """Inspect the producer's serialized v40 state without adding VM hooks."""
    counts = {f'{place}_{role}': 0 for place in ('active', 'parked') for role in ('handled', 'unwinding', 'internal')}
    counts.update(idle='Idle' in state, draining_groups=0, empty_queues=0, active_shields=0, cancelled_scopes=0)
    counts['host_cancellations'] = []

    def count_stack(stack, place):
        for entry in stack:
            if 'unwinding' in entry:
                replay.check(set(entry) == {'value', 'error', 'unwinding'}, 'unknown preserved exception')
                role = 'unwinding' if entry['unwinding'] else 'internal'
            else:
                replay.check(set(entry) == {'C'}, 'handled exception must own a heap value')
                role = 'handled'
            counts[f'{place}_{role}'] += 1

    def walk(value):
        if isinstance(value, list):
            for child in value:
                walk(child)
        elif isinstance(value, dict):
            for key, child in value.items():
                if key == 'vm_state':
                    count_stack(child['exception_stack'], 'active')
                    for task in child['scheduler']['tasks'].values():
                        count_stack(task['exception_stack'], 'parked')
                elif key == 'task_group' and isinstance(child, dict):
                    counts['draining_groups'] += child['state'] == 'Exiting' and bool(child['children'])
                elif key == 'queue' and isinstance(child, dict):
                    counts['empty_queues'] += (
                        not child['getters']
                        and not child['putters']
                        and not child['values']
                        and child['reserved'] is None
                    )
                elif key == 'cancel_scope' and isinstance(child, dict):
                    active = child['entered'] and not child['exited']
                    counts['active_shields'] += active and child['shield']
                    counts['cancelled_scopes'] += active and child['cancel_called']
                elif key == 'native_cancel' and isinstance(child, dict):
                    counts['host_cancellations'].append({'reason': child['reason'], 'delivered': child['delivered']})
                walk(child)

    walk(state)
    return counts


def require_state(actual, requirements):
    for name, minimum in requirements.get('minimum', {}).items():
        replay.check(type(actual[name]) is int and actual[name] >= minimum, f'missing baseline state: {name}')
    for name, expected in requirements.get('equal', {}).items():
        replay.check(replay.equal(actual[name], expected), f'wrong baseline state: {name}')


def load_manifest(path):
    manifest = replay.strict_json(path.read_text())
    replay.check(manifest['schema'] == 1 and manifest['format'] == FORMAT, 'unsupported baseline manifest')
    replay.check(len(manifest['snapshots']) == 6, 'incomplete baseline inventory')
    for item in manifest['snapshots'].values():
        snapshot = path.parent / item['file']
        replay.check(snapshot.parent.resolve() == (path.parent / 'snapshots').resolve(), 'snapshot outside inventory')
        replay.check(replay.digest(snapshot) == item['sha256'], 'baseline snapshot changed')
        header = snapshot.read_bytes()[:8]
        replay.check(header == b'MONTY\0' + FORMAT.to_bytes(2, 'little'), 'wrong producer snapshot version')
    return manifest


def capture(output, association, native, fixtures):
    """Produce review evidence, never a frozen compatibility promise or a committed fixture."""
    configuration = replay.strict_json((ROOT / 'baseline.json').read_text())
    snapshots = output / 'snapshots'
    snapshots.mkdir()
    items = {}
    for name, case in configuration.items():
        directory = output / ('capture-' + name)
        evidence = []
        if case.get('idle'):
            directory.mkdir()
            snapshot = directory / 'idle.bin'
            view = replay.invoke(native, ['start-idle', fixtures / case['source'], snapshot], evidence)
            replay.check(replay.equal(view, {'idle': True}), 'idle producer')
            continuation = (fixtures / case['continuation']).read_text()
            view = replay.invoke(native, ['feed-idle', snapshot, fixtures / case['continuation']], evidence)
            replay.check(replay.equal(view, {'complete': 'ok'}), 'idle producer continuation')
            item = {'idle': True, 'continuation': continuation, 'expected': [view], 'batches': []}
        else:
            result = replay.run_case(case, native, fixtures / case['source'], directory)
            replay.check(result['status'] == 'PASS', f'baseline producer {name}: {result}')
            recorded = replay.strict_json((directory / 'evidence.json').read_text())
            continuous = replay.strict_json(recorded['commands'][-1]['stdout'])
            batches = replay.strict_json((directory / 'batches.json').read_text())
            index = case['capture']
            snapshot = directory / f'cut-{index}.bin'
            item = {
                'idle': False,
                'step': case.get('step', False),
                'expected': continuous[index + 1 :],
                'batches': batches[index:],
            }
        state = census(replay.invoke(native, ['inspect', snapshot], evidence))
        require_state(state, case['requires'])
        target = snapshots / (name + '.bin')
        shutil.copyfile(snapshot, target)
        items[name] = item | {
            'file': str(target.relative_to(output)),
            'sha256': replay.digest(target),
            'source': case['source'],
            'source_sha256': replay.digest(fixtures / case['source']),
            'capture': case.get('capture', 'idle'),
            'census': state,
            'requires': case['requires'],
        }
        (directory / 'inspection.json').write_text(json.dumps(evidence, indent=2) + '\n')
    producer = replay.strict_json(association.read_text())
    producer['receipts'] = {
        name: {'sha256': digest, 'receipt': replay.strict_json((association.parent / name).read_text())}
        for name, digest in producer['receipts'].items()
    }
    producer['git'] = {
        'head': git('rev-parse', 'HEAD'),
        'branch': git('branch', '--show-current'),
        'status': git('status', '--porcelain=v1'),
        'merge_head': git('rev-parse', '-q', '--verify', 'MERGE_HEAD', optional=True),
    }
    producer['versions'] = {
        tool: subprocess.check_output([tool, '--version'], env=guard.environment(), text=True).strip()
        for tool in ('rustc', 'cargo', 'node', 'python3')
    }
    manifest = output / 'baseline.json'
    manifest.write_text(
        json.dumps(
            {
                'schema': 1,
                'status': 'candidate',
                'format': FORMAT,
                'producer_backend': 'native',
                'producer': producer,
                'snapshots': items,
            },
            indent=2,
        )
        + '\n'
    )
    load_manifest(manifest)
    return manifest


def verify(manifest_path, output, commands):
    """Consumers resume the retained producer bytes; they never recompile the scenario."""
    manifest = load_manifest(manifest_path)
    results = {}
    for backend, command in commands.items():
        for name, item in manifest['snapshots'].items():
            key = backend + '-' + name
            directory = output / key
            directory.mkdir()
            evidence = []
            original = manifest_path.parent / item['file']
            replay.check(
                replay.equal(replay.invoke(command, ['dispose', original], evidence), {'disposed': True}), 'dispose'
            )
            for branch in range(2):
                current = original
                if item['idle']:
                    continuation = directory / f'continuation-{branch}.py'
                    continuation.write_text(item['continuation'])
                    actual = replay.invoke(command, ['feed-idle', current, continuation], evidence)
                    replay.check(replay.equal(actual, item['expected'][0]), key + ': idle continuation differs')
                    continue
                replay.check(len(item['batches']) == len(item['expected']) > 0, 'incomplete continuation')
                for index, (batch, expected) in enumerate(zip(item['batches'], item['expected'])):
                    target = directory / f'branch-{branch}-{index}.bin'
                    mode = 'resume-step' if item['step'] else 'resume'
                    actual = replay.invoke(command, [mode, current, json.dumps(batch), target], evidence)
                    replay.check(replay.equal(actual, expected), key + ': continuation differs')
                    if 'complete' not in actual:
                        replay.check(target.is_file(), 'missing continuation snapshot')
                        disposed = replay.invoke(command, ['dispose', target], evidence)
                        replay.check(replay.equal(disposed, {'disposed': True}), 'partial abandonment')
                        current = target
                replay.check('complete' in item['expected'][-1], 'unterminated baseline')
            replay.check(replay.digest(original) == item['sha256'], 'producer bytes changed during replay')
            (directory / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
            results[key] = {'status': 'PASS', 'independent_restores': 2}
            print(key, results[key], flush=True)
    load_manifest(manifest_path)
    (output / 'baseline-report.json').write_text(json.dumps(results, indent=2) + '\n')


def git(*args, optional=False):
    result = subprocess.run(['git', *args], cwd=guard.REPO, text=True, capture_output=True, check=not optional)
    return result.stdout.strip()
