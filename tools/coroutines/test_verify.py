"""Small regressions for the trace driver and real native/Wasm adapters."""

import sys
import tempfile
import trace as traces
import unittest
from pathlib import Path
from unittest.mock import patch

from verify import BASELINE, REPO


class VerifierTests(unittest.TestCase):
    """Keep previously demonstrated false passes visible without a sealing campaign."""

    def test_strict_numbers(self):
        """Numeric kinds, large integers, duplicate keys and nonfinite values stay exact."""
        for left, right in [('1', '1.0'), ('true', '1'), ('9007199254740993', '9007199254740992')]:
            self.assertFalse(traces.matches([traces.strict_json(left)], [traces.strict_json(right)], None))
        for invalid in ['NaN', 'Infinity', '1e400', '{"x":1,"x":2}']:
            with self.assertRaises(ValueError):
                traces.strict_json(invalid)

    def test_missing_and_disposal_failures(self):
        """Missing executables and non-disposal events cannot pass the driver."""
        result = traces.execute([str(REPO / 'playground/missing-coroutine-adapter')], 1)
        with self.assertRaises(ValueError):
            traces.event_from(result)
        with tempfile.TemporaryDirectory(dir=REPO / 'playground') as scratch:
            root = Path(scratch)
            (root / 'cases').mkdir()
            (root / 'cases/case.py').write_text('pass')
            adapter = root / 'bad-disposer.py'
            adapter.write_text(
                'import pathlib, sys\n'
                "if sys.argv[1] == 'start':\n"
                "    pathlib.Path(sys.argv[-1]).write_bytes(b'dump')\n"
                '    print(\'{"effect":1}\')\n'
                'else:\n'
                '    print(\'{"complete":1}\')\n'
            )
            with patch.object(traces, 'ROOT', root):
                _, _, error = traces.run_case(
                    [sys.executable, str(adapter)], {'fixture': 'case.py', 'replies': [{}]}, root, 5
                )
            self.assertEqual(error, 'restored disposal executed guest work')

    def test_report_cannot_be_overwritten(self):
        """A claimed report fails before executing any cases and preserves its bytes."""
        with tempfile.TemporaryDirectory(dir=REPO / 'playground') as scratch:
            report = Path(scratch) / 'existing.json'
            report.write_text('owned evidence\n')
            result = traces.execute(
                [sys.executable, str(Path(__file__).with_name('verify.py')), '--report', str(report)], 5
            )
            self.assertNotEqual(result['exit'], 0)
            self.assertEqual(report.read_text(), 'owned evidence\n')

    def test_actual_adapter_argument_shape_and_recovery(self):
        """Both fresh artifacts reject extra positional/keyword args; valid cuts recover."""
        commands = [
            [str(BASELINE / 'target/debug/coroutine-baseline')],
            ['node', str(BASELINE / 'wasm.mjs')],
        ]
        for command in commands:
            for call in ["exchange('1', 'ignored')", "exchange('1', extra='ignored')", 'exchange(1)', "exchange('1')"]:
                with (
                    self.subTest(command=command, call=call),
                    tempfile.TemporaryDirectory(dir=REPO / 'playground') as scratch,
                ):
                    root = Path(scratch)
                    (root / 'cases').mkdir()
                    (root / 'cases/case.py').write_text(call + "\n'2'\n")
                    with patch.object(traces, 'ROOT', root):
                        trace, _, error = traces.run_case(command, {'fixture': 'case.py', 'replies': [{}]}, root, 20)
                    expected = [{'effect': 1}, {'reply': {}}, {'complete': 2}]
                    self.assertEqual(traces.matches(trace, expected, error), call == "exchange('1')")


if __name__ == '__main__':
    unittest.main()
