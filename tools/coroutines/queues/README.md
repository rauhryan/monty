# Native guest queue candidate

Bounded FIFO storage and waiter selection live in Rust. Python fixtures call
`asyncio._queue(capacity)` (integer 1 through 4294967295, excluding bool), `await asyncio._queue_put(q, value)` and
`await asyncio._queue_get(q)` directly. Operations are lazy, single-delivery,
and must be wrapped in a coroutine for gather. Tokens are opaque and are not
awaitable themselves. No Python coordinator, task selection or queue storage.

Each queue serializes delivery through one reservation. Eligible getters take
precedence over putters; each waiter list is FIFO. The item/put remains owned
until the scheduler activates the task after its cancellation checkpoint.
Logical cancellation releases the reservation and wakes the next waiter;
physical disposal detaches all abandoned routes without running cleanup.

`queue_gate.py` contains the initial literal oracle (written before candidate
code). `supplement.py` adds independently authored reservation, scope exit and
cancelled-put cases. No candidate output generates expectations. The original
module was mechanically renamed from gate.py to avoid colliding with the reused
concurrent_io gate import; expectation bytes stayed unchanged. The supplement's
import was adjusted for that rename before execution.

Build and run from the repo with fresh output paths:

```sh
.venv/bin/python tools/coroutines/queues/bind.py --output playground/coroutine-implementation/queues1-build
.venv/bin/python tools/coroutines/queues/run.py --association playground/coroutine-implementation/queues1-build/queues-association.json --output playground/coroutine-implementation/queues1-proof
```

The gate reuses concurrent_io's transports and fresh-process checkpoint runner.
It checks every external FunctionCall/ResolveFutures cut, independent disposal,
two fresh restore branches, unchanged checkpoint bytes and uninterrupted equality
on native and actual Wasm. `reservation.py` exposes both reservation types at
FunctionCall cuts. Private native tests exercise cancelled wake/delivery and
next-waiter progress, plus live cyclic-value reclamation and repeated retention.
There is no private Wasm census or arbitrary-await persistence claim.

Dump version 27 adds queue state; minimum version remains 13. Previous artifacts
and concurrent4 receipts are historical after rebuild. Preserve all historical
coordinator/conformance fixtures and evidence.
