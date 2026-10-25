# Durable generator kernel

This directory is **not ignored**. Generated reports/cuts and build outputs belong
under the ignored `playground/` tree. From the repository root:

```sh
uv run tools/coroutines/verify.py --report playground/coroutine-implementation/NEW-behavior.json
```

Use a never-used report name. This command runs CPython plus the actual native and
Wasm adapters, prints every required case, and exits nonzero for any failure,
unsupported case, missing artifact, timeout or incomplete trace. `expected.json`
and the milestone cases remain fixed. See `artifacts.json` for fresh source/binary
receipts. All ten cases remain required, including nested delegation, throw, completion
identity and cleanup recovery. See the latest campaign note for recorded results.

`trace.py` retains strict JSON/numeric comparison, rejects duplicate/nonfinite
numbers, and uses a new process for each cut, disposal and resume. Each producer
exits; successors receive only trusted bytes and one accepted host reply. Both
adapters require exactly one positional text argument to `exchange`, no keywords.
CPython supplies language semantics and makes no snapshot claim. The artifact
receipt binds the actual artifact hashes to `source_identity()` in `verify.py`;
stale artifacts cannot pass. No recursive environment/manifests freeze is needed.

The reviewed `review/heap-generator.patch` is now applied: Generator trace/destruction dispatch in `heap/mod.rs`, extended only for Exception
and failed ExternalFuture/GatherFuture owners using their matching edge walkers.
The compile gate is removed. No unsafe, allocator/reader invariant, collector
algorithm or finalizer work was added. Heap slot ceilings remain native 80 / Wasm 56.

The runtime supports lazy generator binding, direct next/send/throw activation, nested
generator delegation, exact completion identity, and rejected
first-send/reentry/recursion cleanup and retry, suspension and exhaustion.
Dormant state moves its locals, operands, namespace and caught exceptions into one
saved segment. Active snapshots and parked tasks transfer activation ownership with
their frames. Physical abandonment runs no guest cleanup. Body StopIteration is
converted to RuntimeError after body handlers decline it.

`yield from` requires a generator delegate. For-loop/native iterator consumption,
saved method attributes, `__next__`, close, generator lambdas and async generators
remain unsupported. `throw` accepts one exception instance or built-in exception
class. Owning errors use contextual clone/drop/walk operations, including failed
future caches. Public graph export rejects owned exception args transactionally;
escaped errors cross as metadata diagnostics without guest formatting. No new
cancellation/gather policy was introduced.

All Cargo goes through the existing serialized lock/budget guard, using the shared
target, \<=2 jobs, incremental=0 and dev/test debug=0. Useful focused commands:

```sh
python3 playground/coroutine-baseline/build.py native --report playground/coroutine-implementation/NEW-native.json
python3 playground/coroutine-baseline/build.py wasm --report playground/coroutine-implementation/NEW-wasm.json
python3 playground/coroutine-baseline/build.py component --report playground/coroutine-implementation/NEW-component.json
python3 playground/coroutine-baseline/build.py generator-runtime-tests --report playground/coroutine-implementation/NEW-runtime.json
python3 playground/coroutine-baseline/build.py dump-compat-tests --report playground/coroutine-implementation/NEW-dumps.json
uv run tools/coroutines/test_verify.py
```

Run builds sequentially, without concurrent interpreter edits. `check_source.py`
forwards compiler/schema/type checks to the guard and no longer mutates source.
The focused runtime test uses memory-model checks and inspects live counts/cycles
before heap teardown, plus original/restored contextual abandonment and task parking.
The v16 dump fixture includes delegated state and completion cycles; old v13/v14/v15
bytes remain unchanged and are resumed by focused tests. The verifier regression test exercises
numeric precision/kinds, missing artifacts, bad disposal, report overwrite and real
native/Wasm argument-shape/recovery checks.

The separately named child-scope proof runs the repaired scope7 library over the
accepted generator runtime, with an outer JSON exchange projecting all pending
child operations. From this checkout:

```sh
.venv/bin/python tools/coroutines/scope9.py --report playground/coroutine-implementation/NEW-scope.json
.venv/bin/python tools/coroutines/verify.py --artifacts tools/coroutines/scope9_artifacts.json --report playground/coroutine-implementation/NEW-kernel.json
```

The scope command requires all 16 fixed reference cases on native and actual
Wasm, plus an accepted-acquire/cancel race and a host cancellation-veto probe.
Every exchange dumps and physically drops its producer, independently restores
and disposes a copy, then restores a fresh process for the selected next command.
Aggregate task/frame ownership is captured by the VM dump, with in-guest alias
assertions; the JSON projection is only an observation. Original/restored native
ownership checks live in `crates/monty/tests/generator_runtime.rs`, using the same
library and `scope9_ownership.py`. Run them through the existing guarded
`generator-runtime-tests` action.

`scope9_artifacts.json` reuses accepted repair5 binaries. Its runtime digest and
normalized test-only delta distinguish appended native tests from runtime edits;
no runtime rebuild is claimed. The original kernel receipt and expectations are
preserved. A runtime change requires fresh builds/association before passing.
The host model keeps cancellation, receipts and producer fences outside the VM;
its reconciliation dispositions model external host work, never guest cleanup.
This bounded scripted proof supplies neither a production journal nor exactly-once
external effects. Guest scope metadata remains cooperative; history/results are
retained, arbitrary CPU loops have no graceful interruption, and async syntax is
future work. Wasm traces do not establish internal Wasm refcounts. See workspace
`notes/monty-host-child-scope-integration9.md` for receipts and review limits.


Experimental async syntax is selected by the reserved first-statement compiler
directive `from __monty_host__ import host_coroutines`; no such Python package
is installed. `async10_guest.py` contains the actual `Timebox` class and `cart`
example. `effect(payload)` returns a generator descriptor continuation;
`Timebox.defer(operation, handle)` registers lazy LIFO releases. The explicit
host in `async10_driver.py` queues settlements and advances computation separately.
It latches cancellation, masks repeats during cleanup, and retains ordinary
cleanup failures. The matching CPython test wraps `effect` with `types.coroutine`
and drives `cart(...).__await__()` directly, with no event loop. An independently
written generator reference checks the same full observation sequence.

After fresh guarded native/Wasm/component builds, run:

```sh
.venv/bin/python tools/coroutines/async10.py --report playground/coroutine-implementation/NEW-async.json
.venv/bin/python tools/coroutines/scope10.py --report playground/coroutine-implementation/NEW-async-children.json
```

The second command uses explicitly written async spawn/join bodies with the
unchanged accepted scope7 driver and fixed scope9 scripts, including child drain
and cleanup ordering. The sequential Timebox example does not itself own child
tasks; combining child ownership into its `__aexit__` API remains outside this
slice. Native ownership tests exercise active/dormant async context state,
pending returns/errors, original/restored disposal and default asyncio behavior.
Old artifact receipts remain historical; runtime changes require a new receipt.

## Combined async context and child scope (candidate 11)

Run the separate combined proof with a new report name:

```sh
.venv/bin/python tools/coroutines/combined11.py --report playground/coroutine-implementation/NEW-combined.json
```

`combined11_guest.py` defines the actual `Timebox(state, task, deadline, suppress=False)` library and example parent/child bodies.
Assemble that source with `combined11_driver.py`, as `combined11.guest_source()` does; these are source definitions, not an installed Python package.
The first-statement `from __monty_host__ import host_coroutines` is Monty's compiler directive, not a module providing Timebox.
For example, inside an admitted task:

```python
async def work(state, task):
    async with Timebox(state, task, 50) as tg:
        handle = await tg.operation(['acquire', 'cart'])
        tg.defer('put-cart-away', handle)
        child = await tg.spawn('a', 'normal')
        result = await tg.join(child)
        return result
```

`operation` yields one external descriptor; `spawn` admits a lazy child and returns its ID; `join` delivers its tagged terminal result.
The bounded example task factory chooses `child_body` for admitted child labels/modes; it is not a general callable serialization API.
`defer` only stores an inert operation/handle pair and starts no effect.
The numeric deadline is exposed as host metadata; the host accepts alarms through a separate cancel command.
No VM timer, autonomous runnable loop or event loop is introduced.

Every Timebox owns a distinct persisted scope record, even for nested contexts in one task.
Its async exit closes admission, retains the body's cause, and yields an explicit drain request.
The driver requests cancellation of unfinished owned children and parks that exact parent continuation.
Only after all owned children are terminal does the host get permission to advance it into LIFO resource cleanup.
Child completion makes the parent runnable; it never executes parent code.
The VM retains pending return values and errors inside the suspended async-with unwind.
Each resource operation carries explicit run/task/scope/parent-scope/boundary/phase metadata.
Descriptor names have no cancellation semantics.
Repeated cancellation preserves queued drain/cleanup input, cursor and original cause; ordinary cleanup errors accumulate while later cleanup is attempted.
Join results become consumed only on actual delivery, including both already-completed and wakeup paths.
Unconsumed ordinary child errors and all child cleanup diagnostics remain reportable on scope exit.
Cancellation is task-wide intent addressed through one of that task's scope IDs; a first cancellation closes all its scope admissions and fans out to descendants.
This candidate does not implement independently cancellable sibling lexical scopes inside one task.

`combined11_reference.py` independently spells the async bodies/exit policy with synchronous generators.
The proof compares every full observation to real CPython async code using explicit await iterators, then replays the same host decisions on native and actual Wasm.
Every exchange dumps/destroys the original producer, disposes an independently restored copy without advancement, and resumes a fresh instance.
The existing bounded authority sidecar records cancellation veto, producer epochs and orphan acquisition receipts separately from mutable guest metadata.
The before-teardown native tests include the actual combined library/driver and measure original/restored graph abandonment.
Wasm traces do not establish internal reference counts.
This is cooperative experimental source, with retained/unbounded task, scope and event history; it is not an importable production scheduler or durable authority journal.
Physical disposal does not execute authored cleanup.
Independent review owns acceptance.
