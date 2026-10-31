"""One-command fail-closed quick conformance; no expectation writing capability."""

import argparse
import copy
import hashlib
import json
import shutil
import subprocess
import sys
import types
from datetime import datetime, timezone
from pathlib import Path

from adapter import REPO, ROOT, source
from cases import CASES, MANDATORY


def digest(path):
    """Hash inputs and artifacts, including untracked paths."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def equal(actual, expected):
    """JSON equality preserves numeric types, ordering and absence versus null."""
    if json.dumps(actual, sort_keys=True, allow_nan=False) != json.dumps(expected, sort_keys=True, allow_nan=False):
        raise ValueError({'actual': actual, 'expected': expected})


def validate(case, response):
    """Every input must produce an observation; require the full literal outcome."""
    views = response['views']
    if len(views) != len(case['batches']) + 1:
        raise ValueError('missing/extra observation')
    for view in views[:-1]:
        if view['result'] is not None:
            raise ValueError('premature completion')
    equal(views[-1], case['expected'])


def fixed_reference(case):
    """No coordinator import path exists: execute the three pinned source copies."""
    ns = {}
    body = source(case['mode']).split('\nstate = new_driver(')[0]
    exec(body.replace('from __monty_host__ import host_coroutines\n', ''), ns)
    ns['request'] = types.coroutine(ns['request'])
    ns['adapt'] = lambda body: body.__await__()
    ns['state'] = ns['new_driver'](case['mode'], False)
    return {'views': [json.loads(json.dumps(ns['step'](batch))) for batch in [[], *case['batches']]]}


def pin_receipt(path):
    """Parse exactly the bytes whose digest is retained for the whole run."""
    data = path.read_bytes()
    return {'path': path, 'sha256': hashlib.sha256(data).hexdigest(), 'data': json.loads(data)}


def check_receipt(receipt):
    """Reject receipt rewrites, including command-only or whitespace changes."""
    if digest(receipt['path']) != receipt['sha256']:
        raise ValueError('receipt changed during run: ' + str(receipt['path']))


def association(reference, candidate_receipt=None):
    """Reject changed fixtures, missing cases and stale baseline runtime artifacts."""
    check_receipt(reference)
    if candidate_receipt:
        check_receipt(candidate_receipt)
    lock = reference['data']
    equal(set(CASES) == MANDATORY, True)
    candidate = candidate_receipt['data'] if candidate_receipt else lock
    for name, expected in lock['files'].items():
        if (
            candidate_receipt
            and not name.startswith('tools/coroutines/conformance/')
            and name != lock['reference_receipt']
        ):
            continue
        equal(digest(REPO / name), expected)
    if candidate_receipt:
        for name, expected in candidate['files'].items():
            equal(digest(REPO / name), expected)
        if not candidate['files'] or not candidate['build_receipt']:
            raise ValueError('candidate executable/loaded source/build receipt required')
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=REPO, text=True
    ).splitlines()
    runtime = {
        name: digest(REPO / name)
        for name in sorted(set(names))
        if (name.startswith('crates/') and ('/src/' in name or name.endswith('Cargo.toml')))
        or name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
    }
    equal(hashlib.sha256(json.dumps(runtime, sort_keys=True).encode()).hexdigest(), candidate['runtime_sha256'])
    # Additional known build inputs are associated separately; historical receipts stay intact.
    extra = {str(p.relative_to(REPO)): digest(p) for p in REPO.glob('crates/**/build.rs')}
    extra.update({str(p.relative_to(REPO)): digest(p) for p in (REPO / '.cargo').glob('*') if p.is_file()})
    equal(extra, candidate['build_inputs'])
    equal(shutil.disk_usage(REPO).free >= 10 * 1024**3, True)
    caches = sum(sum(p.stat().st_blocks * 512 for p in Path(t).rglob('*') if p.is_file()) for t in lock['cache_paths'])
    equal(caches <= 8 * 1024**3, True)
    check_receipt(reference)
    if candidate_receipt:
        check_receipt(candidate_receipt)
    return {'binding_sha256': reference['sha256'], 'free': shutil.disk_usage(REPO).free, 'caches': caches}


def negative_controls():
    """Deliberately corrupt observations; none are interpreter acceptance tests."""
    rejected = []
    for name, kind in [
        ('nested', 'dropped-cleanup'),
        ('queued', 'reordered'),
        ('handle', 'no-op-restore'),
        ('drain', 'swallowed-failure'),
        ('nested', 'missing-case'),
    ]:
        response = fixed_reference(CASES[name])
        if kind == 'dropped-cleanup':
            response['views'][-1]['events'].pop(-2)
        elif kind == 'reordered':
            response['views'][-1]['events'][2:4] = reversed(response['views'][-1]['events'][2:4])
        elif kind == 'no-op-restore':
            response['views'][-1] = copy.deepcopy(response['views'][0])
        elif kind == 'swallowed-failure':
            response['views'][-1]['result'][-1] = []
        else:
            response['views'].pop()
        try:
            validate(CASES[name], response)
        except ValueError:
            rejected.append(kind)
        else:
            raise ValueError('negative control accepted: ' + kind)
    for kind, process in [
        ('adapter-exit', subprocess.CompletedProcess([], 1, '{}', '')),
        ('adapter-stderr', subprocess.CompletedProcess([], 0, '{}', 'failure')),
        ('adapter-empty', subprocess.CompletedProcess([], 0, '', '')),
        ('adapter-duplicate', subprocess.CompletedProcess([], 0, '{"views":[],"views":[]}', '')),
        ('adapter-nonfinite', subprocess.CompletedProcess([], 0, '{"views":NaN}', '')),
    ]:
        try:
            decode(process)
        except ValueError:
            rejected.append(kind)
        else:
            raise ValueError('adapter failure accepted: ' + kind)
    # The same exact hash comparison used by association must reject source/artifact drift.
    for kind, actual, expected in [
        ('source-tamper', digest(ROOT / 'cases.py'), '0' * 64),
        ('missing-matrix-case', sorted(CASES)[:-1], sorted(MANDATORY)),
    ]:
        try:
            equal(actual, expected)
        except ValueError:
            rejected.append(kind)
        else:
            raise ValueError('integrity failure accepted: ' + kind)
    return rejected


def decode(process):
    """Reject adapter errors, diagnostics, duplicate keys and nonfinite JSON."""
    if process.returncode or process.stderr:
        raise ValueError('adapter failure: ' + process.stderr)

    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate key: ' + key)
            result[key] = value
        return result

    def invalid(token):
        raise ValueError('invalid number: ' + token)

    result = json.loads(process.stdout, object_pairs_hook=pairs, parse_constant=invalid)
    json.dumps(result, allow_nan=False)
    return result


def main(reference_binding=ROOT / 'binding.json'):
    """Run both actual interpreters and retain exclusively created evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--adapter', help='JSON argv array for a replacement candidate adapter')
    parser.add_argument('--source-directory', type=Path, help='baseline candidate source override; oracle stays pinned')
    parser.add_argument(
        '--candidate-binding', type=Path, help='new adapter/build receipt; never replaces reference binding'
    )
    args = parser.parse_args()
    if args.adapter and not args.candidate_binding:
        parser.error('--adapter requires --candidate-binding with executable and loaded source hashes')
    output = args.output or REPO / 'playground/coroutine-implementation' / datetime.now(timezone.utc).strftime(
        'conformance-%Y%m%dT%H%M%S.%f'
    )
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'FAIL', 'cases': {}}
    try:
        reference = pin_receipt(reference_binding)
        candidate_receipt = pin_receipt(args.candidate_binding) if args.candidate_binding else None
        report['association'] = association(reference, candidate_receipt)
        report['negative_controls'] = negative_controls()
        command = json.loads(args.adapter) if args.adapter else [sys.executable, str(ROOT / 'adapter.py')]
        if args.candidate_binding:
            candidate = candidate_receipt['data']
            equal(command, candidate['command'])
            executable = Path(shutil.which(command[0]) or command[0]).resolve()
            associated = {str((REPO / name).resolve()): sha for name, sha in candidate['files'].items()}
            equal(associated.get(str(executable)), digest(executable))
            equal(digest(REPO / candidate['build_receipt']), candidate['files'][candidate['build_receipt']])
            report['candidate_binding_sha256'] = candidate_receipt['sha256']
        report['command'] = command
        source_dir = args.source_directory or ROOT / 'reference'
        report['candidate_sources'] = (
            {str(source_dir / n): digest(source_dir / n) for n in ('bindings.py', 'kernel.py', 'coordinator.py')}
            if not args.adapter
            else candidate['files']
        )
        report['executable_sha256'] = digest(shutil.which(command[0]) or command[0])
        report['node_sha256'] = digest(shutil.which('node')) if not args.adapter else None
        for name, case in CASES.items():
            oracle = fixed_reference(case)
            validate(case, oracle)
            report['cases'][name] = {}
            for backend in ('native', 'wasm'):
                uninterrupted = None
                for execution in ('uninterrupted', 'restored'):
                    request = {
                        'version': 1,
                        'case': {k: v for k, v in case.items() if k != 'expected'},
                        'backend': backend,
                        'execution': execution,
                        'output': str(output / name / backend / execution),
                    }
                    if args.source_directory:
                        request['source_directory'] = str(args.source_directory.resolve())
                    process = subprocess.run(
                        command, input=json.dumps(request), capture_output=True, text=True, timeout=180
                    )
                    evidence = {
                        'request': request,
                        'exit': process.returncode,
                        'stderr': process.stderr,
                        'stdout': process.stdout,
                    }
                    (output / f'{name}-{backend}-{execution}.json').write_text(json.dumps(evidence, indent=2) + '\n')
                    response = decode(process)
                    validate(case, response)
                    equal(response['views'], oracle['views'])
                    if execution == 'uninterrupted':
                        uninterrupted = response['views']
                    else:
                        equal(response['views'], uninterrupted)
                report['cases'][name][backend] = 'PASS'
                print(name, backend, 'PASS', flush=True)
        report['final_association'] = association(reference, candidate_receipt)
        for path, expected in report['candidate_sources'].items():
            equal(digest(REPO / path), expected)
        check_receipt(reference)
        if candidate_receipt:
            check_receipt(candidate_receipt)
        report['status'] = 'PASS'
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(output, flush=True)


if __name__ == '__main__':
    main()
