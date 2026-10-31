# Dedicated cancellation exception

This additive suite verifies `asyncio.CancelledError`, the identical class returned
by `anyio.get_cancelled_exc_class()`, and separation from ordinary `Exception` and
`ValueError` handlers. It covers host first-reason ownership, caught-but-terminal
host cancellation, protected cleanup, guest level cancellation, shielding, and
Task await/result/exception inspection.

```sh
python3 tools/coroutines/cancelled_error/verify.py \
  --association playground/coroutine-implementation/cancelled-error1/build1/native-async-association.json \
  --output playground/coroutine-implementation/cancelled-error1/new-run
```

The runner authenticates the original build association before and after execution,
pins its additive inputs, and runs each case uninterrupted and from every genuine
suspension snapshot (two independent restores plus disposal). It also runs all 16
unchanged native_async contract entries, substituting only the Wasm admission
adapter that expects cancellation contract 8. Historical adapters stay unchanged.

`migrated/` contains explicit successors of historical regression sources. Only
cancellation handlers and the needed asyncio imports change; ordinary validation,
cleanup-error and saved-body-error handlers retain ValueError. `migrations.json`
records both hashes. Rust ownership tests use these successors. Original fixtures,
frozen gates and evidence remain untouched.

Dump 36/minimum 36 and protocol 9/minimum 9 deliberately reject older contracts.
Old snapshots must run with their original runtime: a saved ValueError handler
cannot be silently reinterpreted as a CancelledError handler. Genuine historical
binary fixtures are preserved and tested for rejection. The new v36 fixture and
live cut tests verify current restores. This does not add task cancellation
counters, `uncancel`, process-kill cleanup, or complete asyncio compatibility.

`guarded_check.py` uses the existing build lock and budget checks for narrow
protocol/bridge checks, avoiding the unrelated worker type-checker build.
