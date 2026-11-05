# Authored Task cancellation

This additive regression checks uncaught guest-authored CancelledError as a Task
outcome, asyncio/AnyIO group sibling progress through queues, caught cancellation,
replacement cleanup failures and exact-origin scope suppression. `reference.py`
runs the same programs on CPython and upstream AnyIO.

Run `verify.py --association <current native-async-association.json> --output <new directory>`
after the established guarded `native_async/build.py` build. Native and actual
Wasm run uninterrupted and with two fresh restores and disposal at every genuine
suspension cut. The verifier authenticates build inputs before and after execution
and preserves source snapshot bytes. It reuses the existing comparator and
cancellation-contract-8 adapter without changing frozen gates.

`public_authored_cancellation_snapshot_ownership` in generator_runtime repeats
these programs three times with live native heap census, including cyclic error
arguments, original and independently restored abandonment. Wasm transport
disposal is not an instrumented heap census. Prior cached outcomes are not
recomputed when loading snapshots from a runtime with the classification defect.
