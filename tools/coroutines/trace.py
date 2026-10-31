"""Strict trace comparison and fresh-process recovery, reused from the baseline."""

import hashlib
import json
import math
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def digest(path):
    """Hash bytes, including ignored playground inputs and generated artifacts."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def strict_json(text):
    """Keep numeric types and reject duplicate keys at every nesting depth."""

    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f'duplicate JSON key: {key}')
            result[key] = value
        return result

    def invalid(value):
        raise ValueError(f'nonfinite JSON value: {value}')

    def finite_float(token):
        value = float(token)
        if not math.isfinite(value):
            invalid(token)
        return value

    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid, parse_float=finite_float)


def finite_dumps(value, **kwargs):
    """Never emit nonstandard nonfinite numbers into evidence or accepted replies."""
    return json.dumps(value, allow_nan=False, **kwargs)


def execute(command, timeout):
    """Preserve diagnostics and reject absent programs and partial timeout output."""
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
        return {'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr, 'execution': 'ran'}
    except subprocess.TimeoutExpired:
        return {'execution': 'timeout'}
    except OSError as error:
        return {'execution': 'missing', 'diagnostic': str(error)}


def event_from(result):
    """No stderr filtering, skip states, missing output, or exit-code normalization."""
    if result.get('execution') != 'ran' or result['exit'] != 0 or result['stderr']:
        raise ValueError(f'unsuccessful execution: {result}')
    event = strict_json(result['stdout'])
    if not isinstance(event, dict) or len(event) != 1 or next(iter(event)) not in ('effect', 'complete', 'disposed'):
        raise ValueError(f'invalid event: {event}')
    return event


def matches(trace, expected, error):
    """Require the entire ordered trace and no execution failure."""
    return (
        error is None
        and bool(expected)
        and finite_dumps(trace, sort_keys=True) == finite_dumps(expected, sort_keys=True)
    )


def run_case(command, case, directory, timeout):
    """Exit each producer before loading bytes in either a disposer or successor."""
    trace, evidence = [], []
    snapshot = directory / 'cut-0.bin'
    argv = [*command, 'start', str(ROOT / 'cases' / case['fixture']), str(snapshot)]
    for index in range(len(case['replies']) + 1):
        result = execute(argv, timeout)
        evidence.append({'command': argv, **result})
        try:
            event = event_from(result)
            trace.append(event)
            if index == len(case['replies']):
                if set(event) != {'complete'}:
                    raise ValueError('expected completion; extra effect or skipped tail')
                return trace, evidence, None
            if set(event) != {'effect'} or not snapshot.is_file():
                raise ValueError('missing effect or snapshot')
            evidence[-1]['snapshot_sha256'] = digest(snapshot)
            disposal = execute([*command, 'dispose', str(snapshot)], timeout)
            evidence.append({'disposal': index, **disposal})
            disposed = event_from(disposal)
            if set(disposed) != {'disposed'} or disposed['disposed'] is not True:
                raise ValueError('restored disposal executed guest work')
            reply = case['replies'][index]
            trace.append({'reply': reply})
            previous = snapshot
            snapshot = directory / f'cut-{index + 1}.bin'
            argv = [*command, 'resume', str(previous), finite_dumps(reply), str(snapshot)]
        except (ValueError, OSError) as error:
            return trace, evidence, str(error)
    raise AssertionError('unreachable')
