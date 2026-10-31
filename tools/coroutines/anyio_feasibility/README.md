# AnyIO feasibility slice

This is milestone 2 evidence, not full AnyIO acceptance. Read CONTRACT.md for the
frozen supported semantics and scope targets. The supplied module is compiled into
Monty at crates/monty/src/modules/anyio.rs and supplied by the trusted built-in
module registry when ordinary guest code imports anyio. No package loader, source
rewriting, experimental import or guest prelude is involved.

The supported surface is create_task_group()/start_soon() with the accepted native
asyncio.run entry. TaskGroup waiting, failure drain, ownership and snapshots reuse
the existing implementation. A captured frozen Python function calls the authored
function with its arguments and admits the resulting coroutine. The new group flag
is defaulted scalar metadata; existing ownership edges remain authoritative.

CancelScope explicitly raises NotImplementedError. Group cancel_scope is absent.
The three scope examples are valid upstream programs; they demonstrate the still
missing level cancellation, shield boundaries and cancellation-origin semantics.
The verifier reports their capability results separately and never scores them as
successful scope implementation. Existing automatic finally protection is a native
platform divergence. RuntimeError tuple aggregates and ValueError host cancellation
remain explicit divergences. No CancelledError or ExceptionGroup implementation is
claimed. Deadlines, streams, task names/start(), anyio.run and backend selection
remain outside the slice.

Run from the repository root:

```sh
uv run --no-project --python 3.14 --with-requirements tools/coroutines/anyio_feasibility/reference-requirements.txt python tools/coroutines/anyio_feasibility/reference.py
python3 tools/coroutines/anyio_feasibility/build.py --output playground/coroutine-implementation/anyio-feasibility1-build
python3 tools/coroutines/anyio_feasibility/verify.py --association playground/coroutine-implementation/anyio-feasibility1-build/anyio-association.json --output playground/coroutine-implementation/anyio-feasibility1-evidence
```

The reference pins CPython 3.14.5 and upstream AnyIO 4.11.0 and checks literal host
calls and pending boundaries. The host-cancel and unshielded failure-drain cases
are explicit platform traces. The new build association binds current source,
new tests, frozen inputs, checker, reused transports and actual loaded artifacts.
No historical association is rewritten or attributed to a later commit.

The verifier reuses the accepted unchanged FunctionCall/ResolveFutures transport
and per-cut runner. Every supported cut is independently disposed, restored twice
in fresh native processes/actual Wasm instances, checked against literal steps,
and compared with uninterrupted execution. Host cancellation terminals additionally
require exact channel schema, first reason, null value/export/interrupted channels
and exactly one ValueError: FIRST primary error. The accepted checker negative
controls also run. Frozen first-milestone fixtures remain unchanged.

For checker-only changes, `build.py --reuse-association BUILD/native-async-association.json --output FRESH`
authenticates the unchanged runtime source, build receipts and loaded artifacts before binding current checker and
author sources in a new association.
It never rewrites the original receipt or accepts changed runtime bytes.
