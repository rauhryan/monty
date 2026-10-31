# Coordinator migration conformance v1

From the owning checkout:

```sh
.venv/bin/python tools/coroutines/conformance/run.py
```

Requires the existing native executable and actual Wasm component; never builds, installs, copies binaries or rewrites expectations.
An exclusive timestamped directory under `playground/coroutine-implementation/` retains raw commands, exit status, stdout/stderr, assembled guest source, snapshots, hashes and `report.json`.
Failure, timeout, missing artifacts, missing cases, altered inputs, stale runtime association and malformed adapter responses fail the command.
Use `--output <unused-directory>` for a named run.
It checks >=10 GiB free and <=8 GiB across the four historically inventoried workspace target/cache paths before and after execution.
It does not discover unrelated caches elsewhere on the machine.

## Reference and acceptance

`reference/{bindings,kernel,coordinator}.py` are byte-for-byte SOURCE copies of the accepted **uncommitted** repair2 implementation.
HEAD `9d7840abd7df6df9ac4e7ce6ec61dadfcc496c78` is context, not the accepted source version.
Receipt `playground/coroutine-implementation/bounded-repair2-association1.json`, SHA256 `1443c36e6e95c92c00ccfe825fcb9ea9da980fab841ae4ef397f686264ce3a9c`, binds:

- bindings: `ab02106041befb63efdc7ff963b62f3348b5e51c06c1da383a2b7ae96dfbe37e`
- kernel: `94433d3f9b5963c3db77d7ac21fc288628688248cc47517e792915ef7144f211`
- coordinator: `952eb393d2de08895422dc478d8476d28382f58b868ba7557a38e442050dd745`

The oracle executes only those explicit files, with CPython coroutine adaptation.
There is no `coordinator1` import or fallback to mutable candidate sources.
`cases.py` independently authors the host scripts and complete literal final public transcript/outcome; it never reads an adapter.
Intermediate blocked-point observations additionally compare against the fixed reference.
The default candidate executes the same frozen Python source inside real native/Wasm interpreters; this is migration-baseline coverage, **not a native Rust coordinator**.
Use `--source-directory tools/coroutines/coordinator1` to test changing supplied Python sources without changing the oracle.

These are accepted custom semantics, not an independently established specification of all desirable Python behavior:

- Host cancellation is sticky, first reason wins including `''`; catching the signal does not undo terminal cancellation.
- Guest cancellation is a one-shot catchable string signal. Guest requests during protected cleanup are discarded, not deferred. Host cancellation preserves protected cleanup inputs.
- Scope exit cancels/drains children before deferred cleanup; cleanup runs in reverse registration order and failures survive.
- Join returns structured value/error/cancellation outcomes; delivery consumes the outcome. Repeat join is unsupported/rejected.
- Spawn returns opaque scope-owned handles. Completed handles retained in an open owner scope support delayed no-op cancellation; wrong-scope cancellation is rejected.
- Bounded mode uses monotonically increasing integer event IDs with a permanent watermark; wrong-run events do not advance it. Accepted inputs and rejected duplicates/stale replies remain observably distinct.
- No asyncio parity, instruction preemption, hostile-guest isolation of the supplied library, host exactly-once journal, general clock/deadline model or production readiness is implied.

A desired semantic change needs a separately reviewed specification and new fixtures; matching this migration suite alone does not establish that it is desirable.

## Small mandatory matrix

Eleven cases: sequential/nested yields and cleanup; capacity-one queue backpressure with B-before-A host completion; empty/repeat host cancellation; guest cancellation; suppression; accepted-but-queued versus already-delivered settlement cancellation; protected cleanup guest masking; completed-child delayed cancellation/repeat join/wrong owner across snapshots; body failure with child drain and cleanup failure; invalid outcome/wrong-run/nonmonotonic/unknown/stale replies.
The queued/delivered cases retain both acceptance events in host order, but only the delivered case records application delivery.
The queue gate is emitted while the second put is backpressured; final authored traces check both committed puts and out-of-order reads.
The handle cut occurs after 80 cooperative checkpoints, without inspecting task records or relying on a Python budget count for the host cut.
The drain case registers parent then parent-extra cleanup in one scope, requires parent-extra first, fails it, and restores before parent cleanup completes.
Its literal outcome retains BODY, the parent's PARENT-AUDIT failure and the drained child's AUDIT failure.

FIFO ready selection, FIFO queue service and ordered batch acceptance are fidelity choices where they affect this suite's public traces.
Every input batch is accepted before any continuation advances.
Host cuts are quiescent (externally blocked or complete), not four-resume/internal instruction-budget boundaries.
Candidates may use different bookkeeping, batch sizes, GC strategy, task storage and instruction counts; they must preserve these externally observable orders.
The 2,000-step pump cap and process timeouts are harness liveness guards, not a portable instruction-count acceptance requirement for Rust.
Internal runnable-cut races, broad interleavings and stress are deliberately outside this quick tier.

## Replaceable adapter API

`--adapter '["/absolute/executable", "optional-script"]' --candidate-binding <new-receipt.json>` replaces `adapter.py`.
One invocation receives one JSON object on stdin and returns one JSON object on stdout; any nonzero exit, stderr, duplicate keys, nonfinite values, missing/extra observations or malformed JSON fails.
Request fields:

- `version: 1`, `backend: "native" | "wasm"`, `execution: "uninterrupted" | "restored"`, `output`: unused evidence directory.
- `case: {mode, batches}` references the pinned authored workload mode and ordered host inputs. Golden expectations are not sent to the adapter.
- The baseline-only `source_directory` override selects three supplied source files.

Response: `{views: [...], evidence: [...], loaded_source_sha256: "..."}`.
There must be an initial blocked observation and one observation after every authored batch.
Each observation is `{events, trace, result}`; events and application trace are cumulative and ordered.
Events are `["request", operation, payload]`, `["settle", operation, "value"|"error", value]`, or `["reject", originalMessage]`.
Outcome is `null` until complete, then `[kind, value, cause, hostCancelReason, cleanupFailures, childOutcomes]` recursively.
This schema describes public outcomes, not Python task dictionaries.
The baseline bridge translates private records; a Rust adapter supplies its own translation and workload API shim.
It must execute the authored programs' operations, queue behavior, scopes and handles; it cannot substitute remembered fixture output.

Operation IDs normalize bijectively by first request order to O0, O1, ... .
All later replies, acceptances and rejections retain that mapping; unknown IDs remain literal.
No trace sorting, outcome dropping, scope-error rewriting or identity collapsing is permitted.
Task/scope IDs and private handle fields are absent; authored handle operations check validity behavior.
The one `owner-exit:S0` cancellation cause is intentionally literal because string signals are currently user-observable; changing that signal requires an explicit contract revision, not generic ID normalization.

A candidate receipt contains `command`, `files` (repo-relative or absolute SHA256 map including the resolved executable, loaded adapter/workload source and build receipt), `build_receipt` (path), `runtime_sha256` and `build_inputs` in the same format as `binding.json`.
The runner checks the current tracked **and untracked nonignored** Rust runtime source/manifests against the runtime digest and the known build.rs/.cargo inputs separately.
A new runtime requires a new genuine build association; do not update the old runtime hash just to silence a mismatch.
The fixed reference/fixture binding remains mandatory with an external adapter.
Both receipts are parsed from the exact bytes hashed at startup; completion rechecks their digests and uses those original parsed associations, including the candidate command.
This detects persistent receipt rewrites during execution, including semantically unchanged byte edits; it does not monitor transient edits that are reverted before checks.
Ignored generated/runtime inputs must be explicitly listed in `files`; the baseline lists actual Wasm component/JS artifacts, shim JS and native adapter source/manifests.
This is actionable artifact association, not a recursive toolchain/environment freeze or a proof that an arbitrary executable honestly implements its declared sources.
Newly discovered build inputs must be added to a new receipt before acceptance.

## Restore and negative evidence

For each backend the baseline runs the workload uninterrupted in one fresh interpreter, then runs an equivalent execution through fresh producer/consumer processes at every semantic cut.
Every producer writes a real snapshot and drops/resets its original suspended interpreter before exiting.
A separate process loads/disposes each snapshot; two further independent consumers restore the same bytes and their observations must match.
The nested case also resumes a divergent `branch` value, disposes that branch, and recovers the original unmodified snapshot with the original input.
Native uses `Dump::load`/drop; Wasm uses component load/reset. Raw commands and snapshot hashes establish these tested actions; no `restore=true` assertion is used.
Uninterrupted equivalence here bypasses host-call suspension by embedding the authored input script inside the guest; it does not exercise a persistent external host transport.
Disposal proves no authored cleanup is driven and subsequent recovery still works; it does not measure physical heap reclamation.

Twelve negative controls reject dropped cleanup, reordered settlement/cancel delivery, no-op-restore observations, swallowed failure, missing observation, failed/stderr/empty/duplicate/nonfinite adapter responses, altered input hash, and missing matrix case.
They exercise verifier failure paths using deliberately corrupted observations/process results, not interpreter acceptance.
A future adapter's evidence and snapshot implementation require independent review: an adapter can fabricate observations, so JSON assertions alone never establish physical restore/dispose coverage.

Focused repair controls can be run after a successful fresh baseline:

```sh
.venv/bin/python tools/coroutines/conformance/controls.py --baseline <baseline-output> --fifo playground/coroutine-implementation/conformance-review1-controls/fifo-candidate --output <unused-control-directory>
```

These require the actual FIFO source mutant to reject on native and Wasm, uninterrupted and restored.
Separate replay controls exercise the full verifier with unchanged temporary receipts, a changed candidate command, and changed reference receipt bytes.
Only temporary receipt copies are mutated; the unchanged replay must pass and both mutations must reject at completion after all cases run.
Replay controls establish verifier behavior only, not interpreter acceptance.

Snapshots are opaque and backend-local; no Python/Rust or native/Wasm byte equality is required.
Cross-version restore is **not tested or waived** here: specify supported producer/consumer versions and retained-code versus migrated semantics, preserve trusted old snapshots, then test that compatibility matrix separately.
Same-version fresh-process recovery is the claim of this quick suite.

## Intentional updates and larger tier

Verification has no update/regenerate flag.
Edit literals only from an explicit semantic decision, review their diff independently of candidate output, and keep the fixed reference unless deliberately versioning it.
After a reviewed fixture/harness change, propose a fresh binding:

```sh
.venv/bin/python tools/coroutines/conformance/bind.py --output playground/coroutine-implementation/conformance-binding-proposal.json --reason 'Reviewed change and rationale'
```

The proposal command uses exclusive creation, never copies candidate output into expectations, and refuses a reference that differs from accepted repair2.
Review the proposal and deliberately replace `binding.json`; retain prior evidence/receipts.
Changing the accepted reference version requires explicitly updating the receipt/version policy too.
Hashes detect accidental drift, not a malicious simultaneous rewrite of fixtures and their binding.

Larger lifecycle stress and native ownership census remain in `coordinator1` as a separate tier.
Use fixed live concurrency, application retention limits, post-warmup growth/spread and return-to-definitions-baseline bounds; do not require Rust to reproduce 146 or 10 old Python heap entries.
Do not reuse the ownership binary unless its Rust source and all five include_str inputs match its build receipt.
This suite does not run that binary, a Rust build, full repository tests, a private Wasm census, allocator/RSS measurements, old-snapshot upgrades or native-coordinator migration.
