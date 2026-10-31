# Synchronous sandbox stepping scaffold

This runnable example wraps the experimental native1 Rust Scheduler/generator path.
It uses native persisted cancellation ownership and compiler-marked cleanup boundaries.
Every advance runs a fresh native or Wasm process, restoring genuine Monty bytes.

## Guest author

`guest.py` contains only authored Python:

```python
async def checkout():
    hold = await reserve('SKU-123', 2)
    try:
        return await charge(hold['total'])
    finally:
        await release(hold['id'])
```

`contracts.py` supplies those async functions and hides the low-level effect generator.
For now the embedding concatenates the supplied bindings and guest source, with the experimental host-coroutine compiler directive first.
These are injected names, not a newly implemented importable standard library.
`_native1()` appears only in the embedding entry point.
Ordinary guest generator operations remain available; this does not globally reinterpret their yields.

## Host author

`host.py` exposes a deliberately small synchronous interface:

```python
sandbox = Sandbox('wasm')
state = sandbox.advance()
# state.request is a descriptor; state.snapshot is opaque Monty bytes.
state = sandbox.advance(state, Reply(state.request['id'], value=host_result))
# Continue until Finished; each call returns before external I/O is performed.
```

The host dispatches allowlisted operation names to its own implementations.
The demo supplies deterministic example results instead of making real purchases.
No runtime callback imports or executes arbitrary host functions from guest descriptors.
Request IDs are local to one snapshot lineage; they are not durable global idempotency keys.

The facade uses temporary transport files and retains only bytes in `Suspended`.
Each subprocess physically drops its restored state on exit; this does not execute authored cleanup.
Authored cleanup runs when a continuation is resumed normally or with an error.
Snapshots must come from a trusted compatible producer; this scaffold provides no authentication or durable storage.
Do not replay host side effects merely because a snapshot can be independently resumed twice.

## Where coordination belongs

```text
Host calls advance(snapshot, reply)
  restore Task/Scheduler and generator frames
  deliver result or exception at the saved await
  execute guest continuation
  guest requests external work
  Rust records a pending future and parks the task
  serialize suspension
Host receives snapshot + request
  perform external work outside Monty
  call advance again
```

This transport supports one outstanding request.
The intended extension is `advance(snapshot, inputs) -> Suspended(snapshot, requests)`:
the internal scheduler runs ready continuations, parks their external waits, and returns a batch when no task can progress internally.
The host chooses which external results arrive, not which guest task runs.
This batch interface, internal-wait handling and a cooperative runnable-work budget are design targets, not implemented by this facade.
A CPU-bound guest also needs execution limits; the transport's 40-second subprocess timeout is only a demo fail-stop, not resumable preemption.

Future native scope coordination should retain these responsibilities:

| Component | Responsibility |
| --- | --- |
| Generator continuation | Execute, suspend, receive values/errors, unwind; retain ordinary explicit generator control |
| Task/Scheduler | Ready/wait state, task ownership, joins, cancellation delivery, protected cleanup |
| Host | External I/O, alarms, persisted snapshots, reply routing and effect journal |

`Sandbox.cancel(state, reason)` requests native sticky cancellation at a suspended native await.
The first reason wins, including an empty string, even if guest Python catches the injected exception and returns.
`advance(state, reply, cancel='repeat')` submits a cancellation and legitimate cleanup reply in one batch.
Cancellation-only turns during cleanup can return the same request with a new snapshot; do not dispatch that operation a second time.
Rust owns the cause, delivery state, revoked original wait and terminal cancelled outcome.
Compiler-marked plain finally and async context exit delegation protect yielding cleanup, including when the first cancellation arrives there.
A first request during cleanup is delivered when outermost protection ends, before ordinary body work or a new request.
Pending returns yield to the signal; an escaping guest error retains precedence.
See `../native1/README.md` for handler and suppression ordering.
Repeated requests do not interrupt cleanup.
The terminal outcome retains the guest result in `value` (this scaffold's JSON text) or the escaped exception in `error`, alongside `reason`.
This is single-task cancellation; child scopes, joins and drain are unsupported.
`Reply(error=...)` injects a ValueError for one operation; it is not host cancellation.
This example uses ordinary `finally`; it does not promise aggregate cleanup failures or task-group semantics.
Host-alarm registration/revocation and scope metadata belong in the future request protocol, not in a hidden clock inside this facade.

## Run

From the repo, with the already-built native1 artifacts:

```sh
.venv/bin/python tools/coroutines/stepping/demo.py --backend native
.venv/bin/python tools/coroutines/stepping/demo.py --backend native --fail-charge
.venv/bin/python tools/coroutines/stepping/demo.py --backend wasm
.venv/bin/python tools/coroutines/stepping/demo.py --backend wasm --fail-charge
.venv/bin/python tools/coroutines/stepping/demo.py --backend native --cancel
.venv/bin/python tools/coroutines/stepping/demo.py --backend wasm --cancel-in-cleanup
```

Success, error and cancellation paths emit `reserve → charge → release → finished`.
The error path retains the pending exception across the release snapshot.
The demo asserts operation order and terminal outcome.
Use current-source native1 native/Wasm build receipts; the demo itself does not trigger a build.
This scaffold does not replace the immutable conformance gate or constitute cancellation/migration acceptance.
