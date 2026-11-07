//! Custom iteration through ordinary suspendable VM frames.

use std::time::Duration;

use monty::{Dump, MontyRun, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyObject, PrintWriter, ResourceLimits, ResourceTracker,
};

use super::support::{check_public_fixture_snapshots, checked_run, new_repl};

#[test]
fn iterator_loop_semantics() {
    checked_run(include_str!("../fixtures/concurrency/iterator_loop/semantics.py"));
}

#[test]
fn iterator_loop_snapshot_ownership() {
    check_public_fixture_snapshots(include_str!("../fixtures/concurrency/iterator_loop/suspension.py"));
}

#[test]
fn iterator_loop_execution_limit() {
    let source = "class Infinite:\n    def __iter__(self):\n        return self\n    def __next__(self):\n        return 1\nfor item in Infinite():\n    pass";
    let mut run = MontyRun::new(source.to_owned(), "limits.py", vec![], CompileOptions::default()).unwrap();
    let limits = ResourceLimits::default().max_feed_duration(Duration::from_millis(50));
    let error = run
        .run(vec![], ResourceTracker::new(limits), PrintWriter::Disabled)
        .unwrap_err();
    assert_eq!(error.exc_type(), ExcType::TimeoutError);
}

/// Cancellation abandons a suspended adapter without calling the iterator's close.
#[test]
fn iterator_loop_cancellation_ownership() {
    let source = include_str!("../fixtures/concurrency/iterator_loop/cancellation.py");
    let mut repl = new_repl();
    repl.feed_run(
        source.strip_suffix("asyncio.run(main())\n").unwrap(),
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Disabled,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let call = repl
        .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
        .unwrap()
        .into_function_call()
        .unwrap();
    let mut progress = call.resume_pending(PrintWriter::Disabled).unwrap();
    for cancel in [Some("FIRST"), Some("SECOND"), None] {
        let bytes = dump("cancel.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        for _ in 0..2 {
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let mut abandoned = copy.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline);
        }
        let pending = progress.into_resolve_futures().unwrap();
        let results = if cancel.is_none() {
            vec![(
                pending.pending_call_ids()[0],
                ExtFunctionResult::Return(MontyObject::string("ok")),
            )]
        } else {
            vec![]
        };
        progress = pending
            .resume_native(results, cancel.map(str::to_owned), PrintWriter::Disabled)
            .unwrap();
        if let ReplProgress::FunctionCall(call) = progress {
            assert_eq!(call.args.arg(0).unwrap().as_str(), Some("cleanup"));
            progress = call.resume_pending(PrintWriter::Disabled).unwrap();
        }
    }
    let (mut repl, result) = progress.into_complete().unwrap();
    assert!(result.py_repr().contains("FIRST"));
    repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline);
}
