"""Literal negative cases independent of guest contracts and real-VM demo."""

import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace

from journal import Crash, Denied, FakeService, Indeterminate, Journal


class JournalTests(unittest.TestCase):
    """Exercise durable admission/replay boundaries with distinct database connections."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name)
        self.journal = Journal(self.path / 'host.db')
        self.service = FakeService(self.path / 'service.db')
        self.run, self.lineage = self.journal.create_run('tenant-a')
        self.context = ('tenant-a', self.run, self.lineage)
        self.cap = self.journal.grant(self.run, 'charge', 'account-1', 42)
        self.request = {'id': 0, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 42}}
        self.cut = self.capture(self.request)

    def tearDown(self):
        self.journal.close()
        self.service.close()
        self.temp.cleanup()

    def capture(self, request, edge='start'):
        """Test-only trusted producer stand-in; real snapshots are tested in demo.py."""
        return self.journal.capture(
            *self.context, SimpleNamespace(snapshot=b'test-producer', backend='test', request=request), edge=edge
        )

    def settle(self, checkpoint=None, request=None, fault=None):
        """Submit a literal request to the journal boundary."""
        return self.journal.settle(*self.context, checkpoint or self.cut, request or self.request, self.service, fault)

    def test_literal_invalid_authority(self):
        """Guessed strings, foreign handles and malformed constraints give no authority."""
        other, _ = self.journal.create_run('tenant-b')
        foreign = self.journal.grant(other, 'charge', 'account-1', 42)
        cases = [
            {'id': 0, 'cap': 'charge:account-1', 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 42}},
            {'id': 0, 'cap': foreign, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 42}},
            {'id': 0, 'cap': self.cap, 'op': 'release', 'args': {'resource': 'account-1'}},
            {'id': 0, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-2', 'amount': 42}},
            {'id': 0, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 43}},
            {'id': 0, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': True}},
            {'id': 0, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}, 'cleanup': True},
        ]
        for i, request in enumerate(cases):
            with self.subTest(request=request), self.assertRaises(Denied):
                self.settle(self.capture(request, str(i)), request)
        self.assertEqual(self.service.count('charge'), 0)

    def test_identity_payload_and_lineage(self):
        """Changing any bound descriptor or pairing a checkpoint to another lineage fails."""
        for request in [
            dict(self.request, cap='forged'),
            dict(self.request, op='release'),
            dict(self.request, args={'resource': 'account-1', 'amount': 1}),
            dict(self.request, id=99),
        ]:
            with self.assertRaises(Denied):
                self.settle(request=request)
        other_run, other_lineage = self.journal.create_run('tenant-a')
        for context in [
            ('tenant-a', self.run, other_lineage),
            ('tenant-a', other_run, other_lineage),
            ('tenant-b', self.run, self.lineage),
        ]:
            with self.assertRaises(Denied):
                self.journal.cut(*context, self.cut)
        self.assertEqual(self.service.count('charge'), 0)

    def test_restart_replay_revocation(self):
        """Durable outcomes survive restart and revocation; new admissions do not."""
        expected = self.settle()
        self.journal.close()
        self.journal = Journal(self.path / 'host.db')
        self.journal.revoke(self.cap)
        self.assertEqual(self.settle(), expected)
        with self.assertRaises(Denied):
            self.settle(self.capture(self.request, 'new'))
        self.assertEqual(self.service.count('charge'), 1)

    def test_crash_windows(self):
        """Uncertain intent never redispatches, including after restarting both adapters."""
        for fault in ('before_dispatch', 'after_dispatch', 'after_completion'):
            cut = self.capture(self.request, fault)
            with self.assertRaises(Crash):
                self.settle(cut, fault=fault)
            self.journal.close()
            self.service.close()
            self.journal = Journal(self.path / 'host.db')
            self.service = FakeService(self.path / 'service.db')
            if fault == 'after_completion':
                self.assertIn('receipt', self.settle(cut))
            else:
                with self.assertRaises(Indeterminate):
                    self.settle(cut)
                if fault == 'before_dispatch':
                    with self.assertRaises(Indeterminate):
                        self.journal.reconcile(*self.context, cut, self.service)
                else:
                    result = self.journal.reconcile(*self.context, cut, self.service)
                    self.assertEqual(self.settle(cut), result)
        self.assertEqual(self.service.count('charge'), 2)

    def test_cancel_release_and_fork(self):
        """Only pregranted reservation release survives cancellation; labels do not help."""
        release = self.journal.grant(self.run, 'release', 'hold-1')
        branch, fork_cut = self.journal.fork(*self.context, self.cut)
        self.assertNotEqual(
            self.journal.cut('tenant-a', self.run, branch, fork_cut)['effect'],
            self.journal.cut(*self.context, self.cut)['effect'],
        )
        self.journal.cancel(self.run)
        self.journal.cancel(self.run)
        with self.assertRaises(Denied):
            self.settle()
        with self.assertRaises(Denied):
            self.journal.fork(*self.context, self.cut)
        with self.assertRaises(Denied):
            self.journal.grant(self.run, 'charge', 'account-1')
        request = {'id': 8, 'cap': release, 'op': 'release', 'args': {'resource': 'hold-1'}}
        cut = self.capture(request, 'release')
        self.assertEqual(self.settle(cut, request), {'released': 'hold-1'})
        self.journal.revoke(release)
        self.assertEqual(self.settle(cut, request), {'released': 'hold-1'})
        self.assertEqual(self.service.count('charge'), 0)

    def test_explicit_branch_grant_and_replay(self):
        """Only a trusted new lineage grant admits another mutation from the same cut."""
        original = self.settle()
        self.assertEqual(self.capture(self.request), self.cut)
        self.assertEqual(self.settle(), original)
        branch, checkpoint = self.journal.fork(*self.context, self.cut)
        forked = self.journal.settle('tenant-a', self.run, branch, checkpoint, self.request, self.service)
        self.assertNotEqual(forked, original)
        self.assertEqual(self.service.count('charge'), 2)
        changed = dict(self.request, args={'resource': 'account-1', 'amount': 1})
        with self.assertRaises(Denied):
            self.settle(request=changed)
        with self.assertRaises(Denied):
            self.capture(changed)

    def test_cancel_restart_and_release_scope(self):
        """Host cancellation and first reason survive restart; release cannot buy or widen scope."""
        release = self.journal.grant(self.run, 'release', 'hold-1')
        self.journal.cancel(self.run, '')
        self.journal.close()
        self.journal = Journal(self.path / 'host.db')
        self.journal.cancel(self.run, 'replacement')
        self.assertEqual(self.journal.owner(*self.context)['reason'], '')
        with self.assertRaises(Denied):
            self.settle()
        cases = [
            {'id': 4, 'cap': release, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}},
            {'id': 5, 'cap': release, 'op': 'release', 'args': {'resource': 'hold-2'}},
            {'id': 6, 'cap': self.cap, 'op': 'charge', 'args': {'resource': 'account-1', 'amount': 1}},
        ]
        for index, request in enumerate(cases):
            with self.subTest(request=request), self.assertRaises(Denied):
                self.settle(self.capture(request, str(index)), request)
        self.assertEqual(self.service.count('charge'), 0)
        self.assertEqual(self.service.count('release'), 0)

    def test_concurrent_duplicate(self):
        """A competing connection sees durable pending and cannot invoke the service."""
        entered, leave = threading.Event(), threading.Event()
        outcomes = []
        path, context, cut, request = self.path, self.context, self.cut, self.request

        class BlockingService(FakeService):
            """Hold the first call between intent and mutation for a deterministic race."""

            def apply(self, key, descriptor):
                entered.set()
                if not leave.wait(5):
                    raise RuntimeError('test timed out')
                return super().apply(key, descriptor)

        def first():
            """Own database connections in the submitting thread."""
            journal, service = Journal(path / 'host.db'), BlockingService(path / 'service.db')
            try:
                outcomes.append(journal.settle(*context, cut, request, service))
            finally:
                journal.close()
                service.close()

        thread = threading.Thread(target=first)
        thread.start()
        try:
            self.assertTrue(entered.wait(5))
            with self.assertRaises(Indeterminate):
                self.settle()
        finally:
            leave.set()
            thread.join(5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(self.settle(), outcomes[0])
        self.assertEqual(self.service.count('charge'), 1)


if __name__ == '__main__':
    unittest.main()
