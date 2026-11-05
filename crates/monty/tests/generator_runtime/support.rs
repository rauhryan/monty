//! Shared execution and live-heap census helpers for the runtime suites.

use monty::{MontyRepl, MontyRun};
use monty_types::{CompileOptions, ResourceLimits, ResourceTracker};

/// Inspect all live owners after GC and before globals or the heap are destroyed.
pub(super) fn checked_run(source: &str) -> monty::RefCountOutput {
    let mut run = MontyRun::new(source.to_owned(), "generator.py", vec![], CompileOptions::default()).unwrap();
    let output = run.run_ref_counts(vec![]).unwrap();
    assert!(
        output.unreachable.is_empty(),
        "unreachable owners: {:?}",
        output.unreachable
    );
    output
}

/// A low GC interval makes abandonment collection observable between feeds.
pub(super) fn new_repl() -> MontyRepl {
    MontyRepl::new(
        "generator.py",
        ResourceTracker::new(ResourceLimits::default().gc_interval(1)),
        CompileOptions::default(),
    )
}
