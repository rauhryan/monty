# Native1: explicit generator entry into the Rust Scheduler

This is a partial experiment, not a native coordinator replacement. `g._native1()`
executes a durable generator on the current Rust Task. Its direct/delegated yields
await native awaitables instead of returning values to a Python driver. Root
completion returns the result directly. Ordinary `next`, `send`, `throw`, bare
`await generator`, and upstream asyncio scheduling policies retain their semantics.

The entry is explicit and experimental, including for a synchronous generator;
it is not a general `__await__` implementation. There is no Python trampoline or
second ready queue. Host-mode `async def`/`async with` are used by the real workload.
In the gather ownership test, separate REPL feeds compile ordinary wrappers and
host-mode bodies into the same intern table and heap; their compiled function
identities persist in snapshots.

## Scope of implementation

The generator activation route remains the durable foundation.
The existing Rust Task and Scheduler own frames, awaiters and future settlement.
There is no Python scheduler or task-selection API.
Physical abandonment releases those owners without running guest cleanup.

`ReplResolveFutures::resume_native(results, cancel, print)` adds an optional host cancellation input.
It admits only a single native main task blocked directly on one external future.
The Task owns the first reason (including an empty string), delivery flag, revoked call ID and protected-wait phase.
These are plain serialized Rust metadata, with no new heap edges.
The existing failure path consumes the cancelled external registration and wakes its waiter once with a catchable ValueError.
Guest exception injection alone does not set cancellation ownership.
Repeated host requests never inject again, and never consume a cleanup reply.
Replies to the revoked ID are ignored; unknown IDs retain the existing API error behavior.
Cancellation is processed before ordinary replies in the same batch.

Compiler ranges mark every emitted `finally` body and sync/async required context exit bookkeeping, including suspended callers.
No names, request labels, exception messages, or Python latch decide protection.
If the first host request arrives during cleanup, Rust records it but preserves that wait and reply.
When the outermost cleanup continuation ends, it delivers a catchable ValueError before ordinary body work or another request.
Compiler-owned checkpoints run after cleanup operands and suppression bookkeeping are settled.
Non-local return/break/continue completes required outer cleanup before delivery; exited inner handlers cannot catch the signal.
A return from a helper beneath a protected caller remains protected.
A pending return is discarded through ordinary exception unwinding; guest catch-and-return after delivery remains possible.
At the remaining handler-search site, pending cancellation replaces the propagating catchable error after required cleanup. It captures that error's bounded metadata and routes the ValueError signal normally, without replaying completed cleanup.
Active matching uses compiler S ownership; an interrupted selector helper may catch locally and continue matching.
After delivery, ordinary handlers, dynamic selectors and recovery remain enabled; host authority checks still apply to their effects.
Sync and async context suppression finishes before delivery. Cleanup replacement before delivery determines the captured error; suppression leaves null.
No aggregate of caught or replaced errors is retained, and no runnable preemption is promised.
Protection excludes context entry and ordinary body/recovery awaits.
Only host cancellation is deferred: legitimate operation errors continue to raise normally.

After a host request, terminal export is a native-owned dictionary with `status='cancelled'`, `reason`, `value`, `error`, `interrupted_error`, and `value_export_error`.
The guest's return is preserved as `value`, even after catch-and-return.
`error` contains the final escaped exception with its traceback, including an uncaught cancellation exception itself; it is not an aggregate cleanup-failure list.
Ordinary uncaught cleanup errors retain this final exception alongside the first cancellation cause.
Exceptions caught or replaced by guest Python retain ordinary Python semantics; this slice invents no aggregate policy.
`interrupted_error` is captured once at delivery: current propagating catchable error, else innermost unresolved selector error, else null. It contains type, nullable stored message (at most 4096 UTF-8 bytes), truncation flag and `metadata-only` representation. It does not retain args or guest heap roots.
Recoverable unsupported return export and depth failures preserve cancellation with stable `unsupported-value` / `export-depth-limit` codes. Internal, uncatchable and terminal export failures remain fatal.
Without host cancellation, return and exception behavior is unchanged.

The experimental native/Wasm transport accepts `native_cancel` and `native_results` in a JSON input envelope.
The worker's optional `native_cancel` field requires protobuf protocol version 8 for policy7 semantics; cancellation-capable parents must not downgrade to older workers.
The minimum remains 5 for old clients omitting cancellation.
The field is absent for ordinary resume-futures calls.
WIT exposes the same optional field as `native-cancel`. The pure required `cancellation-contract` export returns 7.
Both the shared JS worker and this direct adapter check each actual instance before configure or dispatch. Missing, throwing or incorrect exports fail with `MONTY_CANCELLATION_CONTRACT_UNSUPPORTED`; WorkerChannel preserves that code.
The transport's ordinary scalar and `raise` replies remain supported.
Its terminal script convention is JSON text; the transport reports the native outcome dictionary separately.
Cancellation-only turns in cleanup can return `waiting`, retaining the same outstanding request in the stepping facade.

Dump version 23/min13 persists distinct policy7 Code and latch markers, in addition to Q/R/S metadata.
Genuine older ordinary continuations remain supported. New cancellation refuses legacy active/saved code; legacy pending or delivered latches refuse on resume independently of a null diagnostic.
While current cancellation is pending, ordinary/frozen calls and delegated generator segments are admitted before installation. Current delivered recovery does not inherit those pending-only restrictions.
Older fixtures remain historical artifacts; matching trusted producer artifacts are required.
The candidate supports unchanged `nested` and `host` workloads, including the host case's empty reason and repeat-plus-cleanup reply batch.
The full eleven-case gate still fails at `queue`; the remaining nine cases are unsupported.
No frozen oracle, cases, workload, or binding are changed.

Unsupported: native child ownership, cancellation/drain on scope exit, explicit joins, queues, event watermarks, general operation batches, task-group aggregation, bounded lifecycle stress, and runnable preemption.
Scope authoring remains a thin Python shim.
Do not interpret this single-task slice as the complete selected scope contract.
No performance, broad asyncio parity, or private Wasm heap reclamation claim is made.

## Reproduction

Use fresh output names, from the checkout. All builds enter the existing guarded
lock, shared target, <=2 jobs, incremental disabled, debug info disabled. No installs.

```sh
.venv/bin/python tools/coroutines/native1/bind.py --output playground/coroutine-implementation/native1-build-fresh
.venv/bin/python tools/coroutines/native1/cancellation.py --output playground/coroutine-implementation/native-cancellation-fresh
.venv/bin/python tools/coroutines/native1/proof.py --output playground/coroutine-implementation/native1-proof-fresh
.venv/bin/python tools/coroutines/coordinator1/guarded_checks.py generator-runtime-tests --report playground/coroutine-implementation/native1-ownership-fresh.json
.venv/bin/python tools/coroutines/coordinator1/guarded_checks.py async-compat-tests --report playground/coroutine-implementation/native1-async-fresh.json
```

The binding contains the exact adapter command. Pass that JSON array as `--adapter`
and its `binding.json` as `--candidate-binding` to the unchanged conformance runner.
The build association hashes tracked and untracked nonignored runtime sources,
known build inputs, explicit ignored adapter inputs, native and Wasm artifacts,
loaded JS shims, executable interpreters, and real guarded build receipts.
Rebuilding shared artifacts invalidates historical binary receipts; it never changes
historical receipts or fixed reference/cases/binding bytes.

The Rust ownership tests assert live heap entries return to definitions baseline
for independent original/restored graphs at read, audit and close cuts, before
heap teardown. They include aliased cyclic locals and two upstream gather Tasks.
These are live-entry counts, not allocator/RSS or Wasm memory reclamation measures.

The implementation stops for parent-owned independent review.
No subsequent implementation slice is launched here.
