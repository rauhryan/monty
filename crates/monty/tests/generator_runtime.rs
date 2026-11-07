//! Generator and concurrency ownership checks before interpreter/heap teardown.
//! Each module groups related behaviors; all suites share this integration-test binary.
#![cfg(feature = "ref-count-return")]

#[path = "generator_runtime/generators.rs"]
mod generators;

#[path = "generator_runtime/composition.rs"]
mod composition;

#[path = "generator_runtime/iterator_loop.rs"]
mod iterator_loop;

#[path = "generator_runtime/structured_cleanup.rs"]
mod structured_cleanup;

#[path = "generator_runtime/native_cancellation.rs"]
mod native_cancellation;

#[path = "generator_runtime/asyncio.rs"]
mod asyncio;

#[path = "generator_runtime/anyio.rs"]
mod anyio;

#[path = "generator_runtime/support.rs"]
mod support;

#[path = "generator_runtime/baseline.rs"]
mod baseline;
