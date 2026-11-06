# Concurrency source fixtures

These are source inputs consumed by Rust ownership/compiler tests and the
native/Wasm replay harness in `tools/concurrency/`. They are not Python modules.
Host bindings and guest library fragments are supplied by the owning tests.

The original source bodies came from the research branch at
`34519dd69d7f50f825cbe20a83b388169ea29f1f` and were formatted once with Ruff.
Formatting preserves their Python syntax trees and expected outcomes.
The root Ruff configuration excludes this directory from routine formatting and
linting so these test inputs remain stable, including host-injected names.

- `generators`, `generator_scopes`, `async_contexts`, `structured_cleanup`:
    generator compilation, delegated state, context cleanup and retained ownership.

- `coordinator_stress`, `native_cleanup`, `cancellation_export`, `concurrent_io`:
    scheduler lifetime, disposal, cancellation and terminal-export regressions.

- `tasks`, `task_groups`, `cleanup_boundaries`, `public_queues`, `private_queues`:
    Task/group outcomes, protected cleanup, queue reservation/revocation and run drain.

- `cancel_scopes`, `scope_checkpoints`: AnyIO ancestry, shielding and checkpoint roles.

- `public_async`, `cancelled_error`, `authored_cancellation`: the original 14 public programs
    replayed on native and Wasm. `public_async/contract.json` and its hash manifest
    keep the original expected outcomes; the source hashes record the formatted
    fixtures.

- `api_extensions`: gather exception collection, generator loops and close, and queue work accounting.

- `generator_composition`: list/tuple collection, iterable delegation and synchronous context managers.
    Semantic cases run against CPython; the suspension case adds active-frame restore and disposal checks.

- `upstream_merge`: exception chaining across tasks, generator PEP 479 causes and annotation helpers.

New fixtures follow the same one-time formatting and routine Ruff exclusion policy.
The maintained replay inventory contains 22 programs; additional source fixtures support native semantic and ownership tests.

Keep genuine serialized fixtures in the parent directory unchanged. This source
fixture directory does not replace version rejection or compatibility tests.
