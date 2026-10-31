# Monty cleanup inspector

A small standalone counterpart to yo-workflow's execution page, using the experimental Monty Wasm runtime.
Run from the Monty checkout:

```sh
python3 tools/coroutines/unwind_demo/server.py
# Open http://127.0.0.1:4318
```

Click **Start cart → Cancel → Complete operation → Complete operation**.
The pending operation moves from `charge` to `audit-release` to `release`, then finishes cancelled.
Try **Fail operation** during audit: release still runs, the failure is retained, and cancellation remains the final outcome.
Repeated cancellation during cleanup preserves the outstanding cleanup request.
Completing the body normally also executes cleanup.

Every transition runs the existing `tools/coroutines/native1/wasm.mjs` adapter in a new Node process and actual Wasm instance.
The adapter dumps Monty's continuation and resets its original instance before exiting.
Python holds only JSON observations and snapshot files; guest Python runs inside Monty.
**Check snapshot loads** restores and disposes an independent instance without delivering input.
**Discard without cleanup** removes the retained snapshot without invoking authored cleanup.

The guest-authored `cart.py` uses supplied async bindings from `bindings.py`.
The host checks the fixed contract names and argument types in `server.py`.
The workflow enters the Rust scheduler through the experimental `_native1()` bridge; there is no Python coordinator in this demo.
The VM owns cancellation and protects yielding cleanup.
Separate source panels show the authored cart and supplied bindings; the server assembles the entry point and external `exchange` bridge alongside them.
The dependencies are Python 3, Node, and this checkout's existing experimental component artifacts and preview2 shim used by `wasm.mjs`.
There is no automatic build or installation; missing artifacts fail visibly.
This is not compatible with an unmodified upstream Monty release.

This demo deliberately has one shared local run, simulated I/O, and manually triggered cancellation.
The scope's deadline descriptor is visible but does not schedule a real alarm.
There are no child tasks or journal integration here.
HTTP state is lost on server restart; snapshots are private temporary files removed on normal server shutdown.
Only the current snapshot and at most 100 trace entries are retained.
A new run requires completing or explicitly discarding the previous run.

Verify the working runtime without building:

```sh
python3 tools/coroutines/unwind_demo/check.py
```

This covers success, body failure, cancellation, repeated cancellation, cleanup failure, fresh-instance snapshot loading, physical disposal, and stale revisions.

Bindings are assembled into one guest module for this demo, not installed Python imports.
The host-side allowlist is illustrative input validation, not a production capability system or contract SDK.

Open **Inspect snapshot data** to see the format version, full SHA256 and first 4 KiB in hex/ASCII.
Download the complete binary checkpoint while the run is suspended.
The payload is CBOR; this viewer does not decode VM frames or the heap.
