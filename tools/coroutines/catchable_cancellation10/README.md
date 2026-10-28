# Repair10 bounded safety tests

The `cancellation-test-hooks` Cargo feature is deliberate fault instrumentation,
separate from `test-hooks`. Neither is enabled in production worker builds.
`ref-count-return` already enables `test-hooks` for native census tests; it does
not enable the new faults. The new public Rust host API is thread-local, one-shot,
not serialized, and has no guest/protocol entry point. Instrumented Wasm reads
`MONTY_REPAIR10_FAULT` from the trusted WASI environment when creating its Child.
Do not distribute that artifact or mistake its hash for a production worker.

Frozen expectations (before candidate execution):
- uncaught: cancelled/FIRST, ValueError: FIRST, TypeError metadata with null message;
- later: cancelled/FIRST, TypeError: LATER, same immutable diagnostic;
- depth: cancelled/FIRST, null value/error, export-depth-limit, same diagnostic;
- terminal: actual ExportError::Terminal after at least one memoized prefix pin,
  RuntimeError: REPAIR10_EXPORT_TERMINAL, no cancelled Complete;
- delivery-internal / delivery-uncatchable: actual RunError variants inserted at
  exception routing while cancellation is pending; RuntimeError containing
  REPAIR10_INTERNAL / REPAIR10_UNCATCHABLE, no catcher entry, no Complete;
- projection-internal / projection-uncatchable: same variants at cancelled Return
  projection, no Complete. These cases have already legitimately caught FIRST
  before the recovery wait; the fatal error itself must never become a new signal.

Both native original/restored continuations must return to their measured live
baseline after TRACE and normal roots are dropped, BEFORE teardown. Every executed
cleanup/recovery wait has an independent disposal control that must not run guest
cleanup. Export depth is the actual ResourceTracker ceiling lowered to 16 at the
return seam; the 32-level nested list must reach real recursion_guard failure.
Exporter visits and retired memo pins must be positive, with peak pins equal to
retired pins. No sys.setrecursionlimit guest probe is used.

The four explicitly excluded historical native2/4/5/6 functions remain unchanged.
Separately named native7_policy{2,4,5,6}_owner_census functions keep their lifecycle
scenarios under policy7 expectations. The maintained policy7-runtime-regressions
command excludes exactly those four historical functions. Fatal tests have their
own guarded repair10-fatal-tests command with the dedicated feature enabled.

These tests do not simulate allocator exhaustion or transport failure. Hard
allocator termination cannot promise destructors. Export transaction pin retirement
and live native census prove only unwindable paths. Historical compatibility,
parked-stack, decoder and remaining literal/supersession permutations are outside
this round. Core108/supplemental48 and the fixed oracle remain immutable.
