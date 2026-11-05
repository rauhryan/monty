# Concurrency verification

This is the maintained build/replay entry point for the public asyncio and
cancellation regressions. It loads only tracked code. `playground/` holds outputs,
the shared Cargo cache and lock; no script or manifest is loaded from there.

This first cleanup step deliberately reuses the existing native adapter, Wasm
contract-8 adapter, cases and strict comparator under `tools/coroutines/`. Those
sources remain unchanged, including the frozen 16-coordinate native-async gate.
Do not remove that directory yet. This runner also includes the six cancellation
exception coordinates and six authored-cancellation coordinates: 28 total.
It is not the complete generator/ownership/resource-limit test suite.

## Fresh checkout

The runner currently supports Linux (it uses `fcntl`, `/proc`, allocated disk
blocks and process-group termination, matching the established build guard).
The VM itself retains its existing platform support.

Requirements: Python 3.11+, the repository's Rust toolchain/MSRV, the
`wasm32-wasip1` Rust target, Node 20+ and npm. Install dependencies explicitly:

```sh
rustup target add wasm32-wasip1
npm ci --prefix crates/monty-js
```

No Python package installation is needed for this runner. The native verifier's
manifest and lockfile are tracked here; Cargo uses `--locked`. Build dependencies
still need registry access or an already-populated Cargo cache.

```sh
python3 tools/concurrency/run.py sources
python3 tools/concurrency/test_runner.py
python3 tools/concurrency/run.py run --output playground/concurrency-check-1
```

Use a new output directory for each invocation. `run` builds native, actual Wasm,
and its JS component, then checks every case uninterrupted and through genuine
snapshots with two independent restores, original-byte checks and disposal.
All cases must pass. It records exact input/artifact hashes and build receipts;
verification authenticates those before and after running. Commands and raw
backend output are retained beside each snapshot. A source/artifact change
requires a new build association, not repinning an old report.

To split building and replay without rebuilding unchanged artifacts:

```sh
python3 tools/concurrency/run.py build --output playground/concurrency-build-1
python3 tools/concurrency/run.py verify \
  --association playground/concurrency-build-1/association.json \
  --output playground/concurrency-replay-1
```

## Resource safeguards

The runner uses the same `playground/coroutine-baseline/build.lock` and Cargo
`target` directory as the historical guard, taking the lock nonblockingly for the
whole build/replay. It refuses competing Cargo/rustc or target processes. Builds
use two jobs, disabled incremental compilation and disabled dev/test debug data.
It checks at least 10 GiB free and at most 8 GiB combined caches before, during
and after each command. Each build step has a 15-minute timeout; adapter commands
retain the existing 40-second timeout. It never deletes caches automatically.

On a shared machine, include all additional caches charged to the same budget:

```sh
export MONTY_CHECK_EXTRA_CACHES=/path/to/other/cache:/path/to/another/cache
```

The repository's normal `target`, shared concurrency target and historical review
target are always counted. No sibling repository or workspace metadata is needed.
In this research workspace, continue including the two existing Sweatpants caches;
the workspace handoff records their paths. This configuration does not authorize
reading or changing those repositories' source.

The component build uses the existing JS build script and regenerates its existing
checked-in WIT declarations; inspect unexpected generated diffs. Generated
components and shared executables are replaced by builds. Old receipts remain
historical; their artifact hashes are not rewritten.

## Coverage and remaining cleanup

Native ownership tests remain under `crates/monty/tests/`, including their current
fixture includes. This transport suite checks disposal and replay on both native
and Wasm; it does not claim an instrumented Wasm heap census. The fixed gate's
existing cancellation diagnostic policy remains unchanged; the dedicated
cancellation cases retain their stronger exact-terminal assertion.

The old numbered runners, baseline build helper and evidence are retained for
historical reproduction. They are not prerequisites for the commands above.
Moving fixtures into crate tests and consolidating the comparator are separate
steps, with coverage mapping required before removal. The tracked-only export
check and this runner's refusal tests protect the bootstrap boundary during that
migration.
