# Native asyncio hill-climb gate v1

This is the first executable gate for `plans/monty-anyio-authoring.md` in the parent
workspace. It measures ordinary Python execution, not the old experimental dialect.
No interpreter behavior is implemented here. No adapter preloads guest libraries,
inserts `__monty_host__`, rewrites the program or calls `._native1()`.

## Run

From the Monty checkout, build once per runtime/adapter change:

```sh
PATH="$HOME/.cargo/bin:$PATH" python3 tools/coroutines/native_async/build.py \
  --output playground/coroutine-implementation/native-async-build-NEW
```

The existing guarded builder serializes Cargo, reuses targets, limits jobs to two,
and enforces 10 GiB minimum free disk / 8 GiB combined inventoried cache maximum.
It builds the native transport and actual Wasm, then records source/artifact hashes.
No association is minted for stale binaries. Each output directory must be new.

The verification function is:

```sh
python3 tools/coroutines/native_async/verify.py \
  --association playground/coroutine-implementation/native-async-build-NEW/native-async-association.json
```

Exit 0 means all 16 required backend/case coordinates pass. Exit 1 means FAIL,
including infrastructure failure. The JSON report has `score.passed`,
`score.required`, per-case status/reason and evidence paths. Infrastructure failure
may have no score; it is never success. There is no expectation-update mode.

Each case must pass the full local recovery gate to earn a point: real start,
snapshot at every external cut, disposal in another process, two independent
fresh restores, unchanged original snapshot, and uninterrupted equivalence.
The native executable uses Monty directly; Wasm uses the actual component worker.
Host calls are the supplied `exchange(label)` fixture, resolved as external futures.
Only numeric transport IDs are mapped to unique labels for literal comparisons;
call order, pending obligations and completion values remain checked.

The eight cases cover sequential await, reversed gather completion, async resource
context exit, yielding finally after failure, public create_task/results, normal
TaskGroup drain before parent exit, bounded Queue backpressure, and sticky host
cancellation with a repeated signal while cleanup awaits. The cancellation case
checks cleanup request order and terminal status/first reason, not diagnostic field
parity. All raw diagnostics remain in evidence.

Independent language and checker checks:

```sh
python3 tools/coroutines/native_async/reference.py
python3 tools/coroutines/native_async/verify.py --self-check
```

CPython runs the seven standard cases with a host fixture supplying the scripted
replies; it checks final values and request order. It does not validate Monty
snapshots or our terminal host-cancellation contract. The self-check rejects ten
comparator/JSON/cancellation mutations, including bool/numeric coercion, reordered
values, missing cleanup, duplicate keys, nonfinite/overflow numbers, and overwritten
cancellation cause. These are checker controls, not runtime coverage.

## Fixed target and iteration protocol

`contract.json` and `cases/*.py` are literal inputs pinned by
`contract.sha256.json`. They were authored independently of candidate output.
Do not change them to improve a candidate's score. Semantic changes require a
reviewed contract revision with preserved historical evidence. Hashes catch drift;
they cannot protect against a model deliberately rewriting both fixtures and hashes.
Keep oracle and implementation ownership separate during delegated loops.

The build association checks exact source inventories and artifacts before/after
execution, including adapter hashes; the association's own hash is held through
completion. It does not prove an adversarial adapter honest: review adapter changes.
The verifier never builds and never replaces expected values with observations.
Missing artifacts, parse errors, timeouts, bad replies and unequal traces are failures.

Hill climb: inspect report → change compiler/runtime/public APIs → guarded build
with a new association → rerun unchanged gate → review supported semantics and
ownership. No full lint success or full API acceptance can be inferred from the score.

## Baseline and limits

2026-10-29 baseline: 6/16. Sequential awaits, gather and yielding finally pass on
both backends. Ordinary async context managers, create_task, TaskGroup, Queue and
host cancellation fail honestly. The default route rejects host cancellation with
`native cancellation requires a suspended native task`.

This is a bounded first gate, not full asyncio or AnyIO acceptance. Follow-on fixed
gates must cover TaskGroup sibling failure/error aggregation, guest cancellation,
nested cancellation scopes/shielding, late/duplicate replies and AnyIO public APIs.
Those behaviors must not be declared complete merely because v1 reaches 16/16.
Existing runtime suites and before-teardown ownership checks remain complementary.
No heap census, exhaustive interleavings, old-version snapshots, CPU preemption,
external exactly-once effects or production-ready claim is made here.

No commits or pushes are performed by these commands.
