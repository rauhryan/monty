# Supplied `anyio` subset

`import anyio` provides a supplied subset rather than the upstream AnyIO distribution.
Use the supported [asyncio entry](asyncio.md#module-surface); `anyio.run` is absent.
No AnyIO type-checker stubs are supplied.

## Task-group restrictions

Only the task that entered the group can call `start_soon`.
Unlike upstream AnyIO, child tasks cannot add work to the group.
The group can be entered only once.
`start_soon` requires a callable producing a new ordinary coroutine, accepts positional arguments and returns None.
The `name` keyword, `start`, `task_status`, backend selection and the `anyio.abc` classes are absent.
General introspection and signature/error-message parity are unsupported.
`cancel_scope` becomes available when the group is entered; it is absent on restored older groups without that role.

Failures use the actual-exception `RuntimeError` tuple described in the [asyncio group subset](asyncio.md#public-taskgroup-subset).
They do not produce upstream `ExceptionGroup` objects.
Group failure and host/native task cancellation retain the [asyncio platform policies](asyncio.md), including
native automatic cleanup protection and sticky first host termination.
Failure-driven group cancellation does not implement upstream repeated level cancellation.

## Cancellation scope differences

`CancelScope(*, shield=False)` requires an active supported `asyncio.run`.
Deadlines and writable shield attributes are unsupported.
A scope belongs to the task/run that constructs it; another task cannot enter or exit it.
Cancellation requests from another task in the same active run are permitted; requests outside that run are rejected.
Scopes are single-use and must exit in lexical LIFO order.
Unlike this subset, upstream AnyIO 4.11 permits reuse after a completed block.
Cancelling before entry or after exit is permitted; repeated cancellation is idempotent.
Scope flags describe its own cancellation request and actual own-signal suppression.
A caught cancellation followed by normal scope exit does not set `cancelled_caught`.
Ordinary errors, even identically worded errors, are not suppressed.

Guest cancellation repeats at awaits, host function admission and task activation while its source scope is active.
A lexical shield masks ancestor guest scopes, including inherited group ancestry, but never its own cancellation.
Group children inherit ancestry through the group scope, excluding temporary scopes entered later by their owner.
Native Tasks inherit the creator's current ancestry; ending an inherited shield wakes newly cancellable blocked waits.
Exiting the shield exposes pending ancestor cancellation at the next checkpoint.
Native automatic finally protection does not mask guest level cancellation; yielding guest-cancelled cleanup must
explicitly enter a shield. Group exits still drain admitted children before releasing their owner.
Host function calls are checkpoints even when called synchronously; arbitrary synchronous code is not preempted.

`get_cancelled_exc_class()` returns `asyncio.CancelledError`.
Scopes suppress only their own cancellation signal identity, not arbitrary user-raised instances of that class.
Shields do not mask the initial native host cancellation delivery, clear host cancellation, reopen host-cancelled
admission, or advance execution without host input. Native automatic cleanup protection still applies to host cancellation.
Physical disposal abandons execution without running authored cleanup.

Other AnyIO APIs, including deadlines, `fail_after`, `move_on_after`, sleeps, streams, synchronization, threads and
backend modules, are absent.
