//! Shared execution and live-heap census helpers for the runtime suites.

use monty::{Dump, MontyRepl, MontyRun, ReplProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExtFunctionResult, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

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

/// Repeatedly restore and abandon every cut, checking owners before heap teardown.
pub(super) fn check_public_fixture_snapshots(source: &str) {
    let definitions = source.strip_suffix("asyncio.run(main())\n").expect("entry");
    let mut repl = new_repl();
    repl.feed_run(
        definitions,
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Disabled,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    for _ in 0..3 {
        let mut progress = repl
            .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
            .unwrap();
        let mut cuts = 0;
        while !matches!(progress, ReplProgress::Complete { .. }) {
            cuts += 1;
            assert!(cuts <= 30);
            let bytes = dump("extensions.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for _ in 0..2 {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                    .unwrap();
                assert_eq!(abandoned.heap_entry_count(), baseline, "restored cut {cuts}");
            }
            let mut abandoned = progress.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "original cut {cuts}");
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
                copy.into_function_call()
                    .unwrap()
                    .resume_pending(PrintWriter::Disabled)
                    .unwrap()
            } else {
                let pending = copy.into_resolve_futures().unwrap();
                let ids = pending.pending_call_ids().to_vec();
                pending
                    .resume(
                        ids.into_iter()
                            .map(|id| (id, ExtFunctionResult::Return(MontyObject::string("ok"))))
                            .collect(),
                        PrintWriter::Disabled,
                    )
                    .unwrap()
            };
        }
        let (completed, output) = progress.into_complete().unwrap();
        assert_eq!(output, MontyObject::string("\"ok\""));
        repl = completed;
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "completed history: {source}");
    }
}
