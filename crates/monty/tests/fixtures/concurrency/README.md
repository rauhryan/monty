# Concurrency source fixtures

These are source inputs consumed by Rust ownership/compiler tests and the
native/Wasm replay harness in `tools/concurrency/`. They are not Python modules.
Host bindings and guest library fragments are supplied by the owning tests.

The source bodies were relocated byte-for-byte from the research branch at
`34519dd69d7f50f825cbe20a83b388169ea29f1f`. `.py.txt` denotes source test data;
no cancellation handlers, expected outcomes or assertions changed in relocation.
Where a test deliberately adapts an old cancellation handler itself, `_legacy`
distinguishes that original source from its dedicated CancelledError successor.

- `generators`, `generator_scopes`, `async_contexts`, `structured_cleanup`:
    generator compilation, delegated state, context cleanup and retained ownership.
- `coordinator_stress`, `native_cleanup`, `cancellation_export`, `concurrent_io`:
    scheduler lifetime, disposal, cancellation and terminal-export regressions.
- `tasks`, `task_groups`, `cleanup_boundaries`, `public_queues`, `private_queues`:
    Task/group outcomes, protected cleanup, queue reservation/revocation and run drain.
- `cancel_scopes`, `scope_checkpoints`: AnyIO ancestry, shielding and checkpoint roles.
- `public_async`, `cancelled_error`, `authored_cancellation`: the 14 public programs
    replayed on native and Wasm. `public_async/contract.json` and its hash manifest
    are preserved exactly; the runner maps the manifest's original `.py` names to
    the relocated `.py.txt` files without changing their hashes or expectations.

Keep genuine serialized fixtures in the parent directory unchanged. This source
fixture directory does not replace version rejection or compatibility tests.
