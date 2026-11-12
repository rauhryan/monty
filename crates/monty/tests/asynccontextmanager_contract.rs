//! Acceptance gate for ordinary asynccontextmanager syntax over durable generator frames.
//! No ignored or expected-failure cases.

#![cfg(feature = "ref-count-return")]

#[path = "generator_runtime/support.rs"]
mod support;

use std::time::Duration;

use monty::MontyRun;
use monty_types::{CompileOptions, ExcType, PrintWriter, ResourceLimits, ResourceTracker};

#[test]
fn lifecycle_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!("fixtures/concurrency/asynccontextmanager/lifecycle.py"));
}

#[test]
fn exceptions_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!("fixtures/concurrency/asynccontextmanager/exceptions.py"));
}

#[test]
fn propagation_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!("fixtures/concurrency/asynccontextmanager/propagation.py"));
}

#[test]
fn protocol_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!("fixtures/concurrency/asynccontextmanager/protocol.py"));
}

#[test]
fn nesting_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!("fixtures/concurrency/asynccontextmanager/nesting.py"));
}

#[test]
fn entry_failure_ownership_and_restore() {
    support::check_public_fixture_snapshots(include_str!(
        "fixtures/concurrency/asynccontextmanager/entry_failure.py"
    ));
}

#[test]
fn cleanup_obeys_execution_limit() {
    let source = "import asyncio\nfrom contextlib import asynccontextmanager\n@asynccontextmanager\nasync def manager():\n    try:\n        yield\n    finally:\n        while True:\n            pass\nasync def main():\n    async with manager():\n        pass\nasyncio.run(main())";
    let mut run = MontyRun::new(source.to_owned(), "limit.py", vec![], CompileOptions::default())
        .expect("async generator syntax must compile");
    let error = run
        .run(
            vec![],
            ResourceTracker::new(ResourceLimits::default().max_feed_duration(Duration::from_millis(50))),
            PrintWriter::Disabled,
        )
        .unwrap_err();
    assert_eq!(error.exc_type(), ExcType::TimeoutError);
}

// Exercise the shared census helper independently of the async adapter.
#[test]
fn ownership_harness_control() {
    support::checked_run("token = []\ntoken.append(token)\ntoken = None\nNone");
}
