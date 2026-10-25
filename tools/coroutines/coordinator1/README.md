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

This is a bounded executable composition proof, not an importable asyncio library, production security boundary, exactly-once I/O protocol, CPU preemption, or bounded-retention implementation.
Tasks, events and outcomes are retained until disposal.
A synchronous CPU loop or nonterminating cleanup can still require physical disposal.
Physical disposal performs no authored cleanup and says nothing about releasing real external resources.
The replay checks behavior and live guest aliases; it does not measure private Wasm reference counts or provide a new native heap census for the coordinator graph.
