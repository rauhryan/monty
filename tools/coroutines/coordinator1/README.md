# Supplied Python coordinator proof

Run from the repository root (choose an unused report name):

```sh
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/proof.py --report playground/coroutine-implementation/coordinator1-review-a.json
```

This runs the original async definitions on CPython for a language/reference trace, checks literal policy assertions, then compares every complete observation against the actual native and Wasm backends.
Each exchange dumps the whole guest, physically drops the producer, independently restores/disposes, and resumes in a fresh process or instance.
The host supplies batches of operation replies or execution cancellation; it cannot request advancement of a particular guest task.
The supplied Python coordinator owns generators, FIFO readiness, capacity-one queues, join waits, child ownership, and monotonic operation IDs.
A four-resume turn budget produces durable runnable snapshots; it is cooperative and does not limit instructions within a resume.

`bindings.py` supplies `Scope`, operation/queue/join/checkpoint/cancellation requests and authored async-with unwind.
`kernel.py` adapts the previously accepted combined11 scope policy to generic supplied function factories.
`coordinator.py` owns scheduling and input routing.
`workload.py` contains authored async examples.
`proof.py` assembles the guest source and verifies 13 scenarios.
The actual guest uses the existing host-coroutine compiler directive and identity `adapt`; no ordinary-coroutine trampoline or upstream scheduler integration is involved.
CPython alone needs `types.coroutine` and `__await__` to run the same authored syntax.

## Policy and limits

- Host execution cancellation is sticky, catchable, first-reason-wins, including the empty string.
  Reasons must be strings.
  Repeats cannot interrupt protected cleanup.
- Guest `Scope.cancel(child)` is a distinct one-shot catchable signal.
  If it escapes the child, its outcome is cancelled; catching it and returning produces a value.
  It does not latch host cancellation or cancel the whole run.
  Cancellation requests during protected cleanup are masked in this bounded API.
- Signals use `ValueError` strings (`task-cancel:<reason>` for guest signals).
  This is a deliberately bounded representation with possible string collisions, not `CancelledError`, BaseException semantics, or asyncio parity.
- Scope exit closes admission, cancels/drains children, then attempts all deferred cleanup in reverse registration order.
  Original errors and cleanup failures remain in outcomes.
  Children can fail without immediately cancelling siblings; explicit joins return structured outcomes which callers inspect.
  Invalid ownership and closed scopes produce explicit join errors.
  Repeat consumption is rejected as an unsupported operation in this proof; this does not select a future public single-consumer task API.
- Queue wakes reserve eligibility, not delivery.
  A cancelled waiter cannot consume an item or commit its undelivered put.
  FIFO waiter IDs, ready IDs, and pending join delivery survive snapshots.
- Operation identity is the fixture run ID `C1` plus monotonic `B<number>`; repeated projections retain it.
  Duplicate event IDs, stale run IDs, unknown/late operations and unsupported host commands/outcomes produce explicit rejection events.
  This is a closed experimental protocol, not hostile-input validation or production run identity.

The literal checks are independent policy expectations.
Full trace comparison uses CPython running the same supplied library, so it is an independent language backend, **not an independently implemented scheduling-policy oracle**.
No expected trace is copied from Monty output.
Native/Wasm mismatches, missing programs, stderr, timeouts and stale runtime/artifact identities fail the command.
Snapshots are trusted compatible artifacts, with the existing Monty trust contract.

The original full-history mode is an executable composition proof, not an importable asyncio library, production security boundary, exactly-once I/O protocol, CPU preemption, or bounded-retention implementation.
In that mode tasks, events and outcomes are retained until disposal.
A synchronous CPU loop or nonterminating cleanup can still require physical disposal.
Physical disposal performs no authored cleanup and says nothing about releasing real external resources.
The original replay checks behavior and live guest aliases; it does not measure private Wasm reference counts.
The bounded-mode native coordinator census is described below.

## Bounded retention mode (first hardening slice)

`new_driver(mode, retain_history=False)` selects reclamation. The default remains
full-history instrumentation for the original 28 literal proof/race cases.
This is an explicit protocol choice, not a claim that full-history mode is bounded.

A completed child remains joinable until its result is actually sent into the
owner's continuation. Accepted but undelivered join results retain the child.
After consumption, reclamation removes its task record and owning-scope child
entry; child cleanup/descendant diagnostics transfer to the parent first.
Unjoined outcomes remain until the owning scope exits. Scope exit transfers
unconsumed errors and all cleanup/descendant diagnostics before child deletion.
Exited scopes are removed once their children have been released. Active scope
ancestry, pending operations/replies, queue reservations, and the root outcome
remain live. IDs increase monotonically and are never reused. Repeat joins remain
unsupported, including after reclamation (`invalid-join`).

`Scope.spawn` now returns an opaque `ChildHandle` in both modes, replacing the
experimental string return value. Pass that handle to `join` and `cancel`;
diagnostic task IDs (`T<number>`) remain strings but confer no ownership.
Cancellation of an owned completed handle remains a no-op while its scope is
open, even after join consumption and diagnostic eviction. Wrong-scope,
foreign, never-issued and raw-ID arguments remain invalid. A handle stores only
its task ID, completion bit and an alias to an empty scope-identity object, with
no reference to the task, result, scope or runtime state. The scope never retains
completed handles. Identity aliases survive snapshots; numeric allocation ranges
are not authorization. Live handles also require identity with the task's handle.
These are supplied Python internals, not a hostile-guest security boundary:
mutating private handle fields or coordinator state is unsupported.
Callers which treated spawn results as strings must migrate; host observations
and task-ID projections are unchanged. Existing snapshots retain their captured
old code and semantics; this source change does not migrate old executions.
Application-retained handles cost one fixed-size record per retained handle,
plus ID encoding and one identity object per referenced scope, without retaining
completed outcomes or histories.

Application-held results and unconsumed errors are intentionally retained. A
workflow retaining every result/error is not constant-memory. The stress workload
keeps one scalar sum and at most two children, with a capacity-one queue. Event
history is a diagnostic tail of 64 entries, not an audit journal; pending waits
are owned directly by tasks and do not depend on the tail. Bounded mode requires
strictly increasing integer event IDs in accepted batch order and rejects every
ID at or below its watermark. Full-history mode retains its original arbitrary-ID
dedup semantics. Wrong-run messages do not move the watermark. Retired operation
IDs always reject, independently of diagnostic eviction; no tombstone expiry
can make a late reply address a new operation. Unknown and malformed outcome
messages consume their valid event ID, as before.

The bound is on runtime records for fixed concurrency, bounded input batches and
payloads, and bounded application-retained outcomes. Monotonic counters and their
serialized decimal IDs grow logarithmically; snapshots are not claimed to be a
constant byte size for an infinite execution. No diagnostics representing required
unconsumed errors are silently truncated.

Run maintained regressions and stress (choose fresh output paths):

```sh
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/regression_probes.py --output playground/coroutine-implementation/probes-new
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/regression_races.py --output playground/coroutine-implementation/races-new
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/stress.py --output playground/coroutine-implementation/stress-new
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/regression_handles.py --output playground/coroutine-implementation/handles-new
PYTHONPATH=tools/coroutines .venv/bin/python tools/coroutines/coordinator1/regression_owners.py --output playground/coroutine-implementation/owners-new
RUST_TEST_THREADS=1 .venv/bin/python tools/coroutines/coordinator1/guarded_checks.py generator-runtime-tests --report playground/coroutine-implementation/ownership-new.json
```

Stress asserts <=3 task records, <=3 scope records and <=64 events after every
four-resume turn. At 50-cycle boundaries it dumps, independently disposes a
restoration, then resumes another restoration, retaining only rotating snapshots
and representative cuts. Each new cycle first cancels the preceding cycle's
completed handles, including after sample restoration, before replacing them.
It checks a <=32 KiB post-warmup snapshot-size spread,
full observations against CPython, exact scalar outcome, permanent late reply
rejection, and cancel/cleanup ordering and error retention after 2,000 completed
children. The native test measures live heap entries with memory-model checks,
while globals/heap remain alive, and independently abandons original/restored
ready, sample, and protected cleanup graphs. This is live-entry ownership evidence,
not allocator-byte/RSS measurement. The existing Wasm adapter exposes snapshots
and worker dispatch, not private live-entry/refcount introspection. Snapshot size
and record counts are measured on actual Wasm; reset is not a heap census.

`retention_checks.py --output <fresh-directory>` runs all 13 original scenarios
with reclamation, normalizing event IDs to the ordered protocol while preserving
repeated-ID rejection. It compares live behavior and terminal outcomes with the
original literal-checked matrix, then replays all bounded observations on both
backends. The 15 independent maintained controls retain full-history assertions.

`node tools/coroutines/coordinator1/wasm_capacity.mjs <stress-snapshot>` uses the
component instantiator's existing callback to inspect exported linear-memory
capacity before load, after load/resume, and after reset. This measurement is
available without a runtime rebuild. It measures capacity, not live allocations:
Wasm memory does not shrink on reset. No private Wasm ownership census is claimed.

Both maintained independent runners also accept `--bounded`. Their original
literal assertions and reached-state checks run first; each bounded transition
then matches live state, terminal outcomes, and the last 64 original events
(normalizing only input event IDs). This retains the queued-versus-delivered race
controls when task/scope records are reclaimed.
