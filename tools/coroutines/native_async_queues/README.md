# Public asyncio Queue owning suite

This additive suite leaves `native_async` frozen. Authored code uses ordinary
public asyncio, no prelude, private import, private entry or private scope.

Supported API: `Queue(maxsize=0)` (positional or keyword), `put(item)` (including
keyword item), `get()`, `qsize()`, `empty()`, `full()`, read-only `maxsize`.
Positive plain integer/bool sizes through u32max are bounded; nonpositive sizes
are unbounded under the resource limit. Other size objects are rejected. Public
put/get are lazy single-use coroutines, usable directly with create_task, gather
and TaskGroup. No nowait/join/task_done/shutdown/queue exception classes or full
stdlib type/signature/introspection parity is claimed. See asyncio limitations.

The existing Rust queue stores FIFO values and FIFO waiter lists, serializes
reservations and commits on activation after cancellation checks. No internal
queue wait is registered in the host call map. Even eligible operations
cooperate with the scheduler, unlike CPython's immediate fast path. Inspection
reports committed storage; cancelling a reservation never transfers the value.
After delivery, cancellation does not roll the transfer back. Protected cleanup
requires authored producers/consumers to make progress; no deadlock detection.

`contract.json` contains literal boundaries, replies and final values. The
runner reuses the unchanged adapters and strict frozen comparator. Every FunctionCall and ResolveFutures
cut requires original disposal, preserved original snapshot bytes, two independent
fresh restores and uninterrupted equivalence on native and actual Wasm. Host
cancellation additionally uses the existing strict complete-channel checker,
including every uninterrupted terminal. Run the frozen/additive negative controls
separately; hidden guest errors cannot count as cancellation success.

```sh
python3 tools/coroutines/native_async_queues/verify.py \
  --association <fresh-build>/native-async-association.json --output <fresh-output>
python3 tools/coroutines/native_async_queues/reference.py
```

`--native-preflight` is explicitly unassociated development evidence.
`reference.py` compares six standard matching programs on CPython. Cancellation
uses the platform ValueError contract; group aggregation, protected failure drain,
ordinary cleanup barriers and sticky host termination have explicit platform
traces rather than CPython parity claims. Cyclic/mutable identity is asserted
inside the guest. The Rust census independently disposes original plus two fresh
loads at both FunctionCall and ResolveFutures cuts, including queued/reserved
values, four repetitions per program, and checks terminal heap counts before
teardown. Transport Wasm disposal is not an instrumented Wasm heap census.
