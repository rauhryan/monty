//! Generator collection, iterable delegation and synchronous context managers.

use std::time::Duration;

use monty::MontyRun;
use monty_types::{CompileOptions, ExcType, PrintWriter, ResourceLimits, ResourceTracker};

use super::support::{check_public_fixture_snapshots, checked_run};

/// Collection, iterable delegation and context managers retain Python exception identity.
#[test]
fn generator_composition_semantics() {
    let output = checked_run(include_str!(
        "../fixtures/concurrency/generator_composition/semantics.py"
    ));
    assert_eq!(output.counts["token"], 1);
}

/// Repeated adapters and nested frozen calls balance recursion admission.
#[test]
fn generator_composition_recursion_balance() {
    checked_run(
        r"
from contextlib import contextmanager
@contextmanager
def manager():
    yield 1
def delegate():
    yield from [1]
for index in range(1200):
    assert list(delegate()) == [1]
    with manager() as value:
        assert value == 1
None
",
    );
}

/// Frozen helpers remain subject to the VM's terminal execution limits.
#[test]
fn generator_composition_execution_limits() {
    for source in [
        "def values():\n    while True:\n        yield 1\nlist(values())",
        "def values():\n    while True:\n        yield 1\ntuple(values())",
        "import itertools\ndef values():\n    yield from itertools.count()\nfor item in values():\n    pass",
        "from contextlib import contextmanager\n@contextmanager\ndef manager():\n    try:\n        yield\n    finally:\n        while True:\n            pass\nwith manager():\n    pass",
    ] {
        let mut run = MontyRun::new(source.to_owned(), "limits.py", vec![], CompileOptions::default()).unwrap();
        let limits = ResourceLimits::default().max_feed_duration(Duration::from_millis(50));
        let error = run
            .run(vec![], ResourceTracker::new(limits), PrintWriter::Disabled)
            .unwrap_err();
        assert_eq!(error.exc_type(), ExcType::TimeoutError, "{source}");
    }
}

/// A completed manager does not retain the factory closure or its captured values.
#[test]
fn contextmanager_releases_factory_after_enter() {
    let output = checked_run(
        r"
from contextlib import contextmanager
token = []
def factory(captured):
    @contextmanager
    def manager():
        yield captured
    return manager
wrapped = factory(token)
manager = wrapped()
with manager as value:
    assert value is token
wrapped = factory = value = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// Composition preserves active frames and releases every original/restored owner.
#[test]
fn generator_composition_snapshot_ownership() {
    check_public_fixture_snapshots(include_str!(
        "../fixtures/concurrency/generator_composition/suspension.py"
    ));
}
