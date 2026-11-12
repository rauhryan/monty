# Async context-manager acceptance gate

The existing concurrency runner selects this contract with `--suite asynccontextmanager`.
It specifies ordinary `@contextlib.asynccontextmanager` backed by the existing synchronous-generator continuation.
The gate covers the bounded adapter described in [contextlib limitations](../../docs/limitations/contextlib.md).
Unsupported syntax, ignored tests and missing APIs fail the gate.

Run the CPython 3.14 oracle without building Monty:

```sh
.venv/bin/python tools/concurrency/run.py oracle --suite asynccontextmanager \
  --output playground/async-manager-oracle-1
```

This reports `ORACLE PASS (VM checks not run)`.
The oracle supplies an async `exchange` stand-in and checks the same Python definitions,
assertions, result, and exact host-call order. It does not simulate snapshots or terminal host cancellation.

For the fast native ownership and execution-limit loop:

```sh
RUSTUP_TOOLCHAIN=1.98.0 .venv/bin/python tools/concurrency/run.py ownership --suite asynccontextmanager \
  --output playground/async-manager-ownership-1
```

Run the full gate after implementation changes:

```sh
RUSTUP_TOOLCHAIN=1.98.0 .venv/bin/python tools/concurrency/run.py run --suite asynccontextmanager \
  --output playground/async-manager-check-1
```

The full gate checks the oracle and native ownership first, builds fresh artifacts, then replays every case
on native and actual Wasm. A failing ownership gate stops before the expensive builds.
The runner owns the shared guard: do not wrap it in another guarded command.
The two-job limit, disk floor, cache budget and `MONTY_CHECK_EXTRA_CACHES` apply as described in
[the runner guide](README.md). Every output directory must be new.

To reuse a matching build:

```sh
RUSTUP_TOOLCHAIN=1.98.0 .venv/bin/python tools/concurrency/run.py verify --suite asynccontextmanager \
  --association playground/async-manager-check-1/association.json \
  --output playground/async-manager-recheck-1
```

Source, artifact or receipt drift rejects reuse.
An absent API, rejected syntax, failed assertion, timeout, ownership failure, or failed restore makes the gate fail.
Unsupported cases cannot become successful skips. Ownership-only success does not claim replay coverage.

## Acceptance boundary

The six CPython cases cover repeated factory use, yielded object identity, awaited entry/body/exit,
unchanged body exceptions, suppression, replacement exceptions and their causes, failure before entry,
nested LIFO exit, and zero/two yields on normal and exceptional exits.
Protocol violations must still close the suspended generator, including awaited cleanup.
The native suite also checks original/restored abandonment, live ownership after repeated runs, cyclic values,
and execution limits during nonterminating cleanup.

The seventh replay case specifies Monty's host policy separately: first cancellation reason stays terminal,
`Exception` cannot intercept cancellation, cleanup completes before exit, and a second cancellation preserves
its pending reply even when the guest later catches `CancelledError`.
This is not a CPython cancellation-parity claim.

Replay uses genuine dumps, independent restores, disposal, immutable input-byte checks, and equality with an
uninterrupted trace.
Native ownership checks complement replay; Wasm replay does not claim an instrumented heap census.
The build association authenticates sources and artifacts. `oracle.json`, `ownership.json`/`ownership.log`,
and the existing replay `report.json` retain the separate results.

These checks verify observable behavior, not the internal choice of representation.
Code review must still confirm that the adapter uses existing generator frames and cleanup/ownership traversal
rather than introducing a second continuation engine.
General async iteration, async comprehensions, decorator metadata, and event-loop finalizer hooks are outside
this first gate.

Test the gate's refusal paths with:

```sh
.venv/bin/python tools/concurrency/test_runner.py
```
