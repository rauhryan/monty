"""Handwritten contract traces, independent of Monty and reference trace generation."""

import unittest

from reference import Request, Resource, Runtime, Scope, workload


def settle(runtime, contract, value=None, kind='value', budget=100):
    """Host chooses an external operation, never a guest task."""
    oid = next(oid for oid, op in runtime.pending.items() if op['contract'] == contract)
    return runtime.advance([(f'e{runtime.revision}', 'G1', kind, oid, value)], budget)


def finish_child(runtime, name, value, failed=False):
    """Use explicit host inputs for one child's read and two cleanup operations."""
    settle(runtime, f'read:{name}', value, 'error' if failed else 'value')
    settle(runtime, f'audit:{name}')
    return settle(runtime, f'close:{name}')


class ContractTests(unittest.TestCase):
    """A single workload plus delivery/race controls for the proposed contract."""

    def test_ordered_gather_and_internal_wait(self):
        r = Runtime(workload)
        self.assertEqual(r.advance()['status'], 'blocked')
        self.assertEqual(
            r.pending,
            {
                'G1:O0': {'task': 1, 'scope': 'S0', 'contract': 'read:A', 'version': 1},
                'G1:O1': {'task': 2, 'scope': 'S0', 'contract': 'read:B', 'version': 1},
            },
        )
        self.assertEqual(list(r.limit.waiters), [3])
        finish_child(r, 'B', 'b')
        self.assertEqual([op['contract'] for op in r.pending.values()], ['read:A', 'read:C'])
        finish_child(r, 'C', 'c')
        finish_child(r, 'A', 'a')
        self.assertEqual(settle(r, 'close:parent')['outcome'], ('value', ['a', 'b', 'c']))
        self.assertEqual(
            r.trace,
            [
                'read:A',
                'read:B',
                'audit:B',
                'close:B',
                'read:C',
                'audit:C',
                'close:C',
                'audit:A',
                'close:A',
                'close:parent',
            ],
        )
        self.assertEqual(r.limit.permits, 2)

    def test_proposed_collect_all_failure(self):
        r = Runtime(workload)
        r.advance()
        finish_child(r, 'B', 'B-failed', True)
        self.assertFalse(r.tasks[0].done)
        self.assertFalse(r.tasks[2].consumed)
        finish_child(r, 'C', 'C-failed', True)
        finish_child(r, 'A', 'a')
        self.assertEqual(settle(r, 'close:parent')['outcome'], ('error', "['B-failed', 'C-failed']"))
        self.assertEqual([r.tasks[i].consumed for i in (1, 2, 3)], [True, True, True])
        self.assertEqual(
            r.trace,
            [
                'read:A',
                'read:B',
                'audit:B',
                'close:B',
                'read:C',
                'audit:C',
                'close:C',
                'audit:A',
                'close:A',
                'close:parent',
            ],
        )

        # Ordinary cleanup failure must not erase the already delivered body cause.
        r = Runtime(workload)
        r.advance()
        settle(r, 'read:B', 'B-failed', 'error')
        settle(r, 'audit:B', 'audit-failed', 'error')
        settle(r, 'close:B')
        finish_child(r, 'C', 'c')
        finish_child(r, 'A', 'a')
        settle(r, 'close:parent', 'parent-close-failed', 'error')
        self.assertEqual(r.tasks[0].failure.args[0], ["['B-failed']", 'audit-failed', 'parent-close-failed'])
        self.assertEqual(r.cleanup_failures, ['audit-failed', 'parent-close-failed'])

    def test_cancel_waiter_drain_and_cleanup_failure(self):
        r = Runtime(workload)
        r.advance()
        r.advance([('alarm', 'G1', 'cancel', 'S0', 'deadline')])
        self.assertEqual(list(r.limit.waiters), [])
        self.assertEqual([op['contract'] for op in r.pending.values()], ['audit:A', 'audit:B'])
        before = r.boundary()
        self.assertEqual(
            r.advance([('repeat', 'G1', 'cancel', 'S0', 'later')]), {**before, 'revision': before['revision'] + 1}
        )
        settle(r, 'audit:B', 'audit-failed', 'error')
        settle(r, 'close:B')
        settle(r, 'audit:A')
        settle(r, 'close:A')
        result = settle(r, 'close:parent')
        self.assertEqual(result['outcome'], ('cancelled', 'deadline'))
        self.assertEqual(result['cleanup_failures'], ['audit-failed'])
        self.assertEqual(
            r.trace,
            [
                'read:A',
                'read:B',
                'caught:A',
                'audit:A',
                'caught:B',
                'audit:B',
                'close:B',
                'close:A',
                'close:parent',
            ],
        )
        self.assertEqual(r.limit.permits, 2)
        self.assertEqual(r.tasks[1].result, 'caught')
        self.assertEqual(r.tasks[1].cancel, 'deadline')

    def test_budget_ordered_inputs_and_stable_ids(self):
        r = Runtime(workload)
        self.assertEqual(r.advance(budget=1)['status'], 'runnable')
        self.assertEqual(r.boundary()['pending'], {})
        r.advance()
        before = r.pending.copy()
        r.advance([('deadline', 'G1', 'deadline', 'S0', 1234)], budget=0)
        r.advance([('again', 'G1', 'deadline', 'S0', 9999)], budget=0)
        self.assertEqual(r.deadline, 1234)
        self.assertEqual(r.pending, before)
        r.advance(
            [
                ('b', 'G1', 'value', 'G1:O1', 'b'),
                ('a', 'G1', 'value', 'G1:O0', 'a'),
                ('b', 'G1', 'value', 'G1:O1', 'duplicate'),
                ('stale', 'G0', 'value', 'G1:O0', 'bad'),
            ],
            budget=1,
        )
        self.assertEqual(r.trace, ['read:A', 'read:B', 'reject:b', 'reject:stale', 'audit:B'])
        self.assertEqual(r.boundary()['status'], 'runnable')
        r.advance()
        settle(r, 'audit:B')
        settle(r, 'close:B')
        finish_child(r, 'C', 'c')
        settle(r, 'audit:A')
        settle(r, 'close:A')
        settle(r, 'close:parent')

    def test_cancel_reserved_permit_and_queued_join(self):
        r = Runtime(workload)
        r.advance()
        settle(r, 'read:B', 'b')
        settle(r, 'audit:B')
        settle(r, 'close:B', budget=1)  # C owns a reservation, has not entered its body.
        self.assertIs(r.tasks[3].grant, r.limit)
        r.advance([('cancel', 'G1', 'cancel', 'S0', 'stop')])
        settle(r, 'audit:A')
        settle(r, 'close:A')
        settle(r, 'close:parent')
        self.assertEqual(r.limit.permits, 2)
        self.assertEqual(
            r.trace,
            [
                'read:A',
                'read:B',
                'audit:B',
                'close:B',
                'caught:A',
                'audit:A',
                'close:A',
                'close:parent',
            ],
        )

        async def child():
            raise ValueError('owned')

        async def parent(runtime):
            async with Scope(runtime):
                await runtime.gather(child())

        r = Runtime(parent)
        r.advance(budget=2)  # failure queued to parent but not delivered
        self.assertEqual(r.tasks[0].delivery, [1])
        self.assertFalse(r.tasks[1].consumed)
        r.advance([('cancel', 'G1', 'cancel', 'S0', 'stop')])
        settle(r, 'close:parent')
        self.assertFalse(r.tasks[1].consumed)
        self.assertEqual(str(r.tasks[0].failure), "['owned']")
        self.assertEqual(r.boundary()['outcome'], ('cancelled', 'stop'))

    def test_normal_scope_exit_cancels_and_drains(self):
        async def child(runtime):
            async with Resource(runtime, 'child'):
                await Request('op', 'read:child')

        async def parent(runtime):
            async with Scope(runtime):
                runtime.spawn(child(runtime))
                await Request('op', 'gate')
                return 'early-return'

        r = Runtime(parent)
        r.advance()
        settle(r, 'gate')
        self.assertEqual([op['contract'] for op in r.pending.values()], ['audit:child'])
        with self.assertRaisesRegex(ValueError, '^scope-closed$'):
            r.spawn(child(r))
        settle(r, 'audit:child')
        settle(r, 'close:child')
        self.assertEqual(settle(r, 'close:parent')['outcome'], ('value', 'early-return'))
        self.assertEqual(r.trace, ['gate', 'read:child', 'audit:child', 'close:child', 'close:parent'])
        self.assertEqual(r.tasks[1].cancel, 'scope-exit')

    def test_same_activation_result_cancel_order(self):
        for cancel_first in (False, True):
            with self.subTest(cancel_first=cancel_first):
                r = Runtime(workload)
                r.advance()
                events = [('value', 'G1', 'value', 'G1:O0', 'accepted'), ('cancel', 'G1', 'cancel', 'S0', 'stop')]
                r.advance(list(reversed(events)) if cancel_first else events)
                self.assertEqual(
                    r.trace,
                    ['read:A', 'read:B']
                    + (['reject:value'] if cancel_first else [])
                    + ['caught:A', 'audit:A', 'caught:B', 'audit:B'],
                )
                for name in ('B', 'A'):
                    settle(r, f'audit:{name}')
                    settle(r, f'close:{name}')
                self.assertEqual(settle(r, 'close:parent')['outcome'], ('cancelled', 'stop'))

    def test_common_upstream_probe_workload(self):
        # Same authoring workload and literal trace as upstream_probe.mjs.
        # The failure difference is intentional: reference gather is collect-all.
        async def read(name):
            try:
                return await Request('op', 'read:' + name)
            finally:
                await Request('op', 'close:' + name)

        async def parent(runtime):
            return await runtime.gather(read('A'), read('B'), read('C'))

        for failed in (False, True):
            with self.subTest(failed=failed):
                r = Runtime(parent)
                r.advance()
                self.assertEqual(r.trace, ['read:A', 'read:B', 'read:C'])
                settle(r, 'read:B', 'B-failed' if failed else 'b', 'error' if failed else 'value')
                settle(r, 'close:B')
                self.assertEqual(r.boundary()['status'], 'blocked')
                for name, value in [('C', 'c'), ('A', 'a')]:
                    settle(r, 'read:' + name, value)
                    settle(r, 'close:' + name)
                self.assertEqual(r.trace, ['read:A', 'read:B', 'read:C', 'close:B', 'close:C', 'close:A'])
                self.assertEqual(
                    r.boundary()['outcome'], ('error', "['B-failed']") if failed else ('value', ['a', 'b', 'c'])
                )

    def test_single_consumer_proposal_and_deadlock(self):
        async def child():
            return 7

        async def parent(runtime):
            async with Scope(runtime):
                tid = runtime.spawn(child())
                first = await Request('join', [tid])
                try:
                    await Request('join', [tid])
                except ValueError as error:
                    return first, str(error)

        r = Runtime(parent)
        r.advance()
        self.assertEqual(settle(r, 'close:parent')['outcome'], ('value', ([7], 'join-consumed-or-claimed')))

        async def deadlock(runtime):
            await Request('drain', [0])

        r = Runtime(deadlock)
        self.assertEqual(r.advance()['status'], 'failed')
        r.tasks[0].coro.close()


if __name__ == '__main__':
    unittest.main()
