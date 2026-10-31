# Run and ordinary cleanup boundaries

Additive regressions outside the frozen `native_async` gate.
Run `verify.py --association <fresh native-async-association.json> --output <new directory>`.
Every native and actual-Wasm external cut preserves its original bytes, independently
disposes state, resumes twice from fresh loads, and compares with uninterrupted execution.
Host cancellation cases also check every terminal's complete error/value channels.
`--native-preflight` is explicitly unassociated and supplies no artifact acceptance evidence.

The first four cases copy the exact review5 residual sources; the transferred-body
case copies the historical accepted source. Those copied bytes stay unchanged.
Other cases cover helper transfer, recovery before cleanup, ordinary parent dependencies,
normal/error exits, duplicate shared Tasks, cyclic return/error identity, independent run
ownership, sequential and rejected nested runs, and repeated host cancellation/late replies.

Ordinary coroutine-gather obligations belong to the admitting frame and move to its caller
on helper completion. Entry to authored finally/context cleanup cancels and drains directly
admitted unfinished continuations before running authored cleanup. Recovery before that
entry can execute ordinary code needed by the child. An unrelated deeper helper's finally
cannot claim its caller's obligations. Descendants transferred from a completed scheduler
task retain the existing normal-wait policy; cancellation interrupts their body waits.
Normal gathers, normal TaskGroup exits and protected cleanup gathers retain their wait policy.
These waits require child progress or logical cancellation; code after a lifetime barrier
cannot be relied on to make its child finish. This is a lifetime contract, not general
cycle detection or a guarantee that arbitrary mutually dependent authored programs progress.

Public Tasks remain run-owned. An ordinary shared Task wait does not make the Task a lexical
child. Explicit groups and protected public wait maps retain their established contracts.
A late ordinary gathered error does not replace a first error already caught by guest code;
group aggregation and public Task inspection preserve their actual exception objects.
Logical cancellation runs cleanup through host steps; physical disposal abandons it.

Dump 32 defaults the new ownership/entry metadata absent for older producers. Genuine old
fixtures are preserved. Loading old state does not reconstruct a missing run/cleanup boundary
or repair an exceptional-cleanup role the old producer never recorded.
Queue remains missing; this suite does not accept the milestone or claim asyncio/AnyIO parity.
