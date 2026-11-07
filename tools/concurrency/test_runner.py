"""Regression checks for refusal behavior and unchanged semantic comparisons."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import baseline
import guard
import run


class RunnerTests(unittest.TestCase):
    def test_baseline_census_distinguishes_handler_roles(self):
        preserved = {'value': {'C': 1}, 'error': {}, 'unwinding': False}
        state = {
            'vm_state': {
                'exception_stack': [preserved, {'C': 2}],
                'scheduler': {'tasks': {'1': {'exception_stack': [preserved | {'unwinding': True}]}}},
            }
        }
        counts = baseline.census(state)
        self.assertEqual(counts['active_internal'], 1)
        self.assertEqual(counts['active_handled'], 1)
        self.assertEqual(counts['parked_unwinding'], 1)
        self.assertEqual(counts['active_unwinding'], 0)
        with self.assertRaisesRegex(ValueError, 'missing baseline state'):
            baseline.require_state(counts, {'minimum': {'active_unwinding': 1}})
        with self.assertRaisesRegex(ValueError, 'wrong baseline state'):
            baseline.require_state(counts, {'equal': {'idle': True}})

    def test_baseline_refuses_changed_producer_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'snapshots').mkdir()
            items = {}
            for index in range(6):
                path = root / 'snapshots' / f'{index}.bin'
                path.write_bytes(b'MONTY\0' + (40).to_bytes(2, 'little'))
                items[str(index)] = {'file': str(path.relative_to(root)), 'sha256': run.digest(path)}
            manifest = root / 'baseline.json'
            manifest.write_text(json.dumps({'schema': 1, 'format': 40, 'snapshots': items}))
            baseline.load_manifest(manifest)
            (root / 'snapshots/0.bin').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'baseline snapshot changed'):
                baseline.load_manifest(manifest)

    def test_budget_boundaries(self):
        guard.require_budget({'free': 10 * guard.GIB, 'combined': 8 * guard.GIB})
        for measured in (
            {'free': 10 * guard.GIB - 1, 'combined': 0},
            {'free': 10 * guard.GIB, 'combined': 8 * guard.GIB + 1},
        ):
            with self.assertRaises(RuntimeError):
                guard.require_budget(measured)

    def test_shared_lock_is_nonblocking(self):
        with guard.session():
            child = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), '--lock-child'],
                capture_output=True,
                text=True,
                timeout=10,
            )
        self.assertEqual(child.returncode, 0, child.stderr)

    def test_receipt_cannot_be_overwritten(self):
        with tempfile.TemporaryDirectory(dir=guard.STATE) as directory:
            receipt = Path(directory) / 'receipt.json'
            receipt.write_text('original')
            with guard.session(), self.assertRaises(RuntimeError):
                guard.execute([sys.executable, '--version'], receipt)
            self.assertEqual(receipt.read_text(), 'original')

    def test_association_rejects_drift(self):
        with tempfile.TemporaryDirectory(dir=guard.STATE) as directory:
            directory = Path(directory)
            receipt = directory / 'build.json'
            receipt.write_text('receipt')
            path = directory / 'association.json'
            path.write_text(
                json.dumps(
                    {
                        'sources': {'source': 'one'},
                        'artifacts': {'binary': 'two'},
                        'receipts': {receipt.name: run.digest(receipt)},
                    }
                )
            )
            with (
                patch.object(run, 'sources', return_value={'source': 'one'}),
                patch.object(run, 'artifacts', return_value={'binary': 'two'}),
            ):
                run.authenticate(path)
                receipt.write_text('changed')
                with self.assertRaises(RuntimeError):
                    run.authenticate(path)
            with patch.object(run, 'sources', return_value={'source': 'changed'}):
                with self.assertRaises(RuntimeError):
                    run.authenticate(path)
            with (
                patch.object(run, 'sources', return_value={'source': 'one'}),
                patch.object(run, 'artifacts', return_value={'binary': 'changed'}),
            ):
                with self.assertRaises(RuntimeError):
                    run.authenticate(path)

    def test_suite_inventory(self):
        self.assertEqual(len(run.preflight()), 27)
        cases = run.preflight('asynccontextmanager')
        self.assertEqual(len(cases), 7)
        self.assertEqual(sum(case['oracle_labels'] is not None for case in cases.values()), 6)
        original = run.replay.strict_json

        def missing_case(source):
            result = original(source)
            if 'host_cancel' in result:
                del result['host_cancel']
            return result

        with patch.object(run.replay, 'strict_json', side_effect=missing_case):
            with self.assertRaisesRegex(ValueError, 'case inventory changed'):
                run.preflight('asynccontextmanager')

    def test_suite_rejects_aliased_oracle_calls(self):
        original = run.replay.strict_json

        def duplicate_label(source):
            result = original(source)
            if 'host_cancel' in result:
                result['exceptions']['oracle_labels'] = ['caught', 'caught']
            return result

        with patch.object(run.replay, 'strict_json', side_effect=duplicate_label):
            with self.assertRaisesRegex(ValueError, 'aliased oracle call'):
                run.preflight('asynccontextmanager')

    def test_oracle_failure_keeps_evidence(self):
        cases = {'example': {'source': 'example.py', 'result': [], 'oracle_labels': []}}
        for code, stdout in ((1, ''), (0, '{"result": [], "labels": ["unexpected"]}')):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as directory:
                output = Path(directory)
                result = subprocess.CompletedProcess(['oracle'], code, stdout, '')
                with patch.object(run.replay.subprocess, 'run', return_value=result):
                    with self.assertRaises(ValueError):
                        run.oracle(cases, output)
                self.assertEqual(json.loads((output / 'oracle.json').read_text())[0]['exit'], code)

    def test_ownership_requires_all_tests(self):
        for passed, ignored in ((0, 0), (7, 1), (8, 0)):
            with self.subTest(passed=passed, ignored=ignored), tempfile.TemporaryDirectory() as directory:
                output = Path(directory)
                (output / 'ownership.log').write_text(f'test result: ok. {passed} passed; 0 failed; {ignored} ignored;')
                with patch.object(run.guard, 'execute'):
                    if passed == 8:
                        run.ownership(output)
                    else:
                        with self.assertRaisesRegex(ValueError, 'all eight tests'):
                            run.ownership(output)

    def test_frozen_negative_controls(self):
        run.preflight()


if __name__ == '__main__':
    if sys.argv[1:] == ['--lock-child']:
        try:
            with guard.session():
                raise SystemExit('lock unexpectedly acquired')
        except BlockingIOError:
            raise SystemExit(0)
    guard.STATE.mkdir(parents=True, exist_ok=True)
    unittest.main()
