# Concurrent host I/O gate

`gate.py` compares eight frozen milestone observations.
`expected.json` retains its pre-implementation SHA-256, `bda3743bc9baba932d48786adf0f65b41385c0c941061100b05a72e658224c43`.
Absent observations fail; the historical eleven-case migration gate is untouched.
The three negative controls test the comparator, not runtime mutations.

`proof.py` exercises the accepted gather slice: two Rust tasks, reverse host replies, input-ordered results.
`proof2.py` adds explicit joins, unconsumed child errors, independent guest cancellation and scope draining.
Both transports run the Rust scheduler; neither has a Python coordinator fallback.
`scope.py` is an authoring context manager: its exit awaits the Rust-owned drain and exposes remaining ordinary errors in `scope.errors`.
A resource context enclosing this scope performs its cleanup after that drain.
The ordinary `child` wrapper enters the dialect generator's `_native1()` continuation.

The private experimental primitives are `_scope()`, `_spawn(coroutine)`, `_cancel(handle)` and `_drain()` in `asyncio`.
Handles are awaitable, belong to a specific task and scope token, and retain their outcome independently of the live task record.
A join consumes its error obligation only on actual delivery; a scheduled wake does not consume it.
Completed success/cancellation records leave the scope immediately; unconsumed ordinary errors remain until delivery or drain.
Cancellation of an owned completed child is a no-op; unknown/foreign handles reject.
A child catching guest cancellation still joins with `ValueError('guest')` if it returns normally.
An ordinary cleanup failure remains an ordinary error instead of being discarded.
Cancellation delivered before a child starts does not enter its body.
Repeated guest cancellation preserves a protected cleanup wait.

Only one scope may be open per task.
Use the context manager: directly abandoning the private primitives physically disposes children, without running guest cleanup.
Owned children blocked on joins or gathers support logical cancellation and scope drain.
Host cancellation still requires the existing single-task contract, so requirements 3 and 6 remain unsupported.
Requirement 7's full duplicate/late/unknown/invalid settlement contract remains unsupported.
Requirement 8 remains incomplete overall; the proofs cover the supported cases at quiescent ResolveFutures cuts.
FunctionCall acknowledgement turns are not individually checkpointed.
Independent physical disposal is not logical cancellation.

Run from the checkout with fresh evidence names:

```sh
.venv/bin/python tools/coroutines/coordinator1/guarded_checks.py concurrent-ownership-tests --report playground/coroutine-implementation/concurrent2-unit.json
.venv/bin/python tools/coroutines/concurrent_io/bind.py --output playground/coroutine-implementation/concurrent2-build
.venv/bin/python tools/coroutines/concurrent_io/proof.py --association playground/coroutine-implementation/concurrent2-build/concurrent-association.json --output playground/coroutine-implementation/concurrent2-gather
.venv/bin/python tools/coroutines/concurrent_io/proof2.py --association playground/coroutine-implementation/concurrent2-build/concurrent-association.json --ownership-report playground/coroutine-implementation/concurrent2-unit.json --round1-observations playground/coroutine-implementation/concurrent2-gather/observations.json --output playground/coroutine-implementation/concurrent2-children
.venv/bin/python tools/coroutines/concurrent_io/gate.py --observations playground/coroutine-implementation/concurrent2-children/observations.json
```

The gate deliberately exits 1 until all eight requirements pass.
Builds reuse the existing guard/lock, shared target, two-job cap and disk budgets.
Receipts preserve source/artifact associations; rebuilding shared binaries invalidates their association with earlier source receipts without rewriting historical evidence.


Round3 adds `proof3.py`, which exercises both FunctionCall acknowledgements and
ResolveFutures suspensions in fresh processes, alongside an uninterrupted replay.
It covers owned-tree host cancellation, protected LIFO deferred cleanup and
first-settlement routing. Invalid batches use Rust's borrowed `validate_results`
API (also called by the shared protocol worker) before consuming a suspension.
New version25 executions use a host-only monotonic call-ID range; settled IDs
need no tombstone history. Genuine older snapshots retain strict pending-ID
validation (and their existing revoked-ID exception), since their counters also
allocated private handles. No cross-backend byte interchange is implied.
Round4 adds `proof4.py` for gather cancellation and first-error lifetime transfer,
nested gathers, protected cleanup, guest isolation and direct external items.
Version26 stores gather lifetime links separately from result delivery.
A failed gather may deliver its first error immediately; its remaining tasks stay
owned until scope drain completes. Undelivered ordinary cleanup failures appear in
`scope.errors`; direct child handles remain restricted to their owning scope.
Older snapshots do not acquire the new owned-gather lifetime guarantee.

The concurrent-I/O settlement contract uses the call IDs issued by the runtime
and acknowledges those same IDs. Caller-assigned `ExtFunctionResult::Future`
IDs from the lower-level legacy API are outside this proof. IDs are scoped to
one execution, not a cross-execution deduplication journal.
