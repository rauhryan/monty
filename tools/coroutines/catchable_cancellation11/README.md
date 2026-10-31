# Compatibility iteration 11

This suite supplements the immutable policy7 oracle.
It does not close full cancellation acceptance.
The bounded handoff is `notes/monty-catchable-cancellation-compatibility11.md` in the coordinating workspace.

`legacy.py` authenticates genuine delivered Dump19/20/21 producer receipts and checks ordinary/repeat refusal on native and actual Wasm.
`historical_build.py` runs the existing shared-lock/budget guard against an unmodified Git archive of `9d7840abd7df6df9ac4e7ce6ec61dadfcc496c78`.
That is a Dump17/Proto6 historical source rebuild, not recovery of the lost exact max7 binary.
Its component action substitutes `component.mjs` for the guard's nominal component command; the wrapper preserves the shared target, lock, limits and process monitor.
`component.mjs` generates separate historical glue/cores without overwriting current component glue.
The Wasm build does replace the shared raw Wasm target: restore production using the normal guarded build/bind workflow afterward.
Do not run this workflow while another build is active.

`historical.mjs` freezes identities before the actual mixed capability call, retains link failure versus capability refusal distinctly, and produces an unmodified trusted Dump17 frozen-cache fixture through matching historical glue.
`channel.mjs`/`channel-worker.mjs` use an actual worker, current serveDispatch/WorkerChannel and historical modules, checking classification, zero requests/subscriptions/ready and completed worker termination.
`client.mjs` transpiles unchanged historical TS sources with the installed TypeScript compiler and runs the historical WorkerTransport configure/feed codec against the current component.
This is a source-rebuilt historical client, not an installed historical npm package.

`activation.py` is literal guest source shared with the appended native integration test and actual-Wasm `activation.mjs`.
The four supported routes are frozen reduce, new generator, dormant generator and mixed saved delegation.
Ordinary/current-delivered controls run bodies; pending refusal leaves markers empty and the same old objects reusable.
The coroutine route in the exploratory Wasm driver remains a retained failing probe: host syntax lowers await to generator-only yield-from and never reaches ordinary Coroutine admission.
Do not count that failure or its cancelled error envelope as compatibility refusal.
The separate native unit test reaches `await_coroutine` directly and checks atomic refusal and ordinary/delivered admission before body execution.

All new VM/repl seams are `cfg(test)` only.
The saved-task test uses production save_task_context, serializes/restores the saved state, and temporarily clones the task's handled PARKED exception into the unrelated VM exception stack during cancel_native.
It retires those temporary owners before normal task activation.
This is explicit host instrumentation, not a naturally scheduled second task or a modified dump.
Original/restored native heaps are within one test process.
There is no actual-Wasm private stack injection or heap census claim.
`diagnostics.py` separately reexecutes immutable current-over-S/handled-outer cases through native and actual Wasm, with two independent restored suffixes per cut.

Evidence scripts create fresh receipts and never overwrite historical snapshots or expected outcomes.
Builds use coordinator1/guarded_checks.py; explicit Python lint uses Ruff with --no-force-exclude and --no-cache.
Full conformance remains FAIL at queue with nine unsupported workloads.
