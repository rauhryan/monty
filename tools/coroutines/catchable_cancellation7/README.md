# Proposed catchable cancellation contract 7

This fresh policy oracle is a design artifact, not an acceptance result for repair6.
It never imports a candidate, builds, or updates expectations. Historical native1,
conformance, and evidence files remain authoritative for their original revisions.

Run from the repository with the existing Python environment:

```sh
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python tools/coroutines/catchable_cancellation7/contract.py --self-check
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python tools/coroutines/catchable_cancellation7/contract.py --emit-inputs
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python tools/coroutines/catchable_cancellation7/contract.py --check /absolute/fresh/observations.json
```

`cases.json` freezes source, host batches and literal expected observations before
runtime edits. `--emit-inputs` excludes expectations. A future independently reviewed
adapter must execute each source using native1's actual native and actual Wasm seams,
with fresh output directories. Do not generate observations by copying expectations.
Every case runs uninterrupted and restored. At every suspended cut, hash genuine
dump bytes, physically dispose the original, resume identical bytes twice in separate
processes, compare both results, and dispose independent branches. Retain raw requests,
responses, source and artifact hashes, version headers and authenticated build receipts.

The check input is a JSON list of records with exactly these coordinates:
`{"case":"body-catch-return","backend":"native","mode":"restored","observed":...}`.
`observed` has `effects` (all preterminal boundary views in order, including repeated
unchanged waits) and `terminal` (the six proposed envelope fields). Parse the guest's
JSON string in terminal `value` into an object. For terminal `error` only, normalize
traceback to the final `Type: message` line, preserving empty messages. Do not normalize
interrupted diagnostics, traces, keys, reasons, missing waits or extra effects. Raw
terminal values and errors remain in receipts. Missing diagnostic fields MUST fail.
Current native1 transport shims only decode text/none envelope entries; implementation
must add safe graph decoding for the structured summary before using these cases.
No adapter or runtime execution is claimed by this artifact.

The 27 source cases are the executable semantic core, not the complete acceptance
matrix. The workspace design `notes/monty-catchable-cancellation-design7.md` additionally
specifies exact review-source replay, identity, heap, export, admission, authority and
resource gates. CPython compilation here checks syntax only; it does not validate
native awaits or host cancellation. No claim of asyncio parity is made.

Repair8 adds strict JSON type/key validation and duplicate-key rejection before
conversion. `tests/negative_controls.py` independently exercises malformed records;
run it with the same existing Python and PYTHONDONTWRITEBYTECODE=1. `gates8.md`
freezes supplemental acceptance tables and the separate all-cut manifest requirement;
`supersession8.md` maps obsolete error-first expectations. Neither is a runtime pass.
Original contract/README/cases bytes and historical raw batch bytes are preserved in
`playground/coroutine-implementation/catchable-cancellation-repair8/originals/`.
The workspace repair8 amendment supersedes the design7 Wasm guard proposal; original
design7/review7 evidence and the core cases.json are unchanged.
