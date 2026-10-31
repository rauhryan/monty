# Milestone 2 bounded contract (frozen before candidate implementation)

Reference: CPython 3.14.5, AnyIO 4.11.0, asyncio backend. This is a supplied
implementation, not the upstream distribution or a general AnyIO backend.

Executable supported slice: ordinary `import anyio`; `create_task_group()`;
`async with`; `start_soon(async_callable, *args)` returns None and admits only
in the active owning task. The existing public asyncio.run is the entry.
Normal exit waits children; failure cancels/drains siblings before parent release.
Actual exceptions survive in the existing RuntimeError tuple aggregate (upstream
uses ExceptionGroup). Names, start(), task_status and backend selection are deferred.
start_soon is restricted to ordinary async functions producing a new coroutine;
no task handle is exposed. No host effect is synthesized by the adapter.

Host cancellation retains the native first-reason sticky terminal policy and
ValueError signal. Authored finally cleanup yields under host control and repeated
cancellation preserves cleanup replies. This is platform behavior, not upstream
AnyIO scope shielding. No claim of CancelledError support is made.

Scope feasibility target, intentionally separate from the supported slice:
`with CancelScope() as scope`, explicit cancel(), cancel_called/cancelled_caught,
level cancellation on each cancellable wait, synchronous owning-task enter/exit,
LIFO nested scopes and child scope inheritance. Inner shield=True masks outer
cancellation until leaving the shield; shielded yielding cleanup still needs host
advancement. Scope exit suppresses only its own cancellation, never ordinary errors
or a host terminal decision. TaskGroup.cancel_scope must use the same ownership
model. These require a task-owned scope stack and cancellation-origin state;
coalesced native task cancellation and automatic finally protection are insufficient.
This slice MUST reject CancelScope explicitly rather than provide a no-op scope.
The reference scope programs run unchanged on upstream and candidate: upstream
produces the desired trace; candidate produces an exact unsupported capability
record before any target effects. That record is evidence of a missing seam, not
successful scope implementation. Full scope/AnyIO acceptance remains red.

No deadlines, streams, sleeps, custom scope DSL, external package loader, wall clock,
Playground or scheduler replacement. No source rewriting or injected guest prelude.
New state must preserve old dumps using named/defaulted metadata and existing roots.
The target traces in contract.json are literal external expectations; references
must compare against them. Candidate observations must not generate expectations.

Upstream preflight correction before any candidate execution: unshielded yielding
cleanup on sibling failure is cancelled again by upstream level cancellation.
The original literal target is preserved in preflight-target evidence. `failure`
therefore compares two immediate actual errors with upstream; `failure-drain`
retains the original yielding-drain target as an explicit native platform policy.
It is not upstream equivalence. The scope target specifies the missing shield.

The separate coalesced-seam probe demonstrates the existing public Task behavior:
once guest cancellation is caught, a second unshielded wait is admitted. This
compares against CPython asyncio as a substrate probe, not AnyIO scope parity.
The level scope target above instead requires both cancelled waits to be rejected.

Candidate preflight fixture repairs preserve the original files in evidence:
restore failure-drain's literal child-release step from the pre-candidate target
(the split accidentally shared a mutable list), and assert actual exception classes
with isinstance plus exact repr. Monty's type(exc) is BuiltinException identity
check is independently false even for the preserved actual exception; exact repr
and isinstance avoid that unrelated substrate identity limitation. No external
target trace or required exception content is derived from candidate observations.

The final author sources declare the host callable under typing.TYPE_CHECKING.
This branch does not execute or replace the host input; no lint-suppression comments are used.
These declarations and formatting do not change the literal external targets.
