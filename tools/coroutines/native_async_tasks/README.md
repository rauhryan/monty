# Public asyncio Task regression traces

Additive tests for `create_task`, cached result/exception identity, aliasing,
multiple pending joins (including duplicate gather slots), discarded handles,
creator completion, guest cancellation, joined-child cleanup, caught cancellation,
and protected gather first-error ordering. No experimental scope or prelude.

Run `python3 tools/coroutines/native_async_tasks/verify.py --association <fresh-build>/native-async-association.json --output <new-directory>`.
Every coordinate uses the unchanged fixed-gate checker and real native/Wasm adapters,
including independent disposal, repeated fresh restoration at every cut and
uninterrupted equivalence. These cases do not alter the fixed 16-coordinate gate.
Guest cancellation currently raises ValueError; it does not establish CancelledError
parity or host cancellation of concurrent ordinary tasks.
