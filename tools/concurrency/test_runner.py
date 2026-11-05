"""Regression checks for refusal behavior and unchanged semantic comparisons."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import guard
import run


class RunnerTests(unittest.TestCase):
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
