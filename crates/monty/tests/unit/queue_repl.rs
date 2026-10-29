//! Live queue/task records and restored abandonment, measured before REPL teardown.
use super::*;
use crate::{Dump, Session, SessionRef, dump};

/// Repeated cancelled and ordinary scopes release cycles, futures and scheduler records.
#[test]
fn ownership_queue_scope_records_and_abandonment() {
    for host_cancel in [false, true] {
        let mut repl = MontyRepl::new("queues.py", ResourceTracker::default(), CompileOptions::default());
        for source in [
            include_str!("../../../../tools/coroutines/concurrent_io/wrappers.py"),
            include_str!("../../../../tools/coroutines/concurrent_io/scope.py"),
        ] {
            repl.feed_run(source, vec![], PrintWriter::Disabled).unwrap();
        }
        let definitions =
            include_str!("../../../../tools/coroutines/queues/ownership.py").replace("parent()._native1()", "");
        repl.feed_run(
            &definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        repl.heap.collect_cycles();
        let baseline = repl.heap.entry_count();
        for _ in 0..24 {
            let mut progress = repl
                .feed_start("parent()._native1()", vec![], PrintWriter::Disabled)
                .unwrap();
            let mut first = true;
            let mut saw_parent = false;
            loop {
                let mut parent_cleanup = false;
                while let ReplProgress::FunctionCall(call) = progress {
                    parent_cleanup |= call.args.arg(0).unwrap().as_str() == Some("parent-cleanup");
                    progress = call.resume_pending(PrintWriter::Disabled).unwrap();
                }
                if matches!(progress, ReplProgress::Complete { .. }) {
                    break;
                }
                let bytes = dump("queues.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned.heap.collect_cycles();
                assert_eq!(
                    abandoned.heap.entry_count(),
                    baseline,
                    "restored abandonment before teardown"
                );
                let pending = progress.into_resolve_futures().unwrap();
                if parent_cleanup {
                    assert_eq!(pending.vm_state.ownership_child_counts(), (1, 0, 0));
                    assert_eq!(pending.vm_state.ownership_gather_counts(), (0, 0, 0, 0));
                    saw_parent = true;
                }
                let results = if host_cancel && first {
                    vec![]
                } else {
                    pending
                        .pending_call_ids()
                        .iter()
                        .map(|id| (*id, ExtFunctionResult::Return(MontyObject::none())))
                        .collect()
                };
                progress = pending
                    .resume_native(results, host_cancel.then(|| "FIRST".into()), PrintWriter::Disabled)
                    .unwrap();
                first = false;
            }
            assert!(saw_parent);
            let (mut completed, _) = progress.into_complete().unwrap();
            completed.heap.collect_cycles();
            assert_eq!(
                completed.heap.entry_count(),
                baseline,
                "completed cycle before teardown"
            );
            repl = completed;
        }
        eprintln!("queue scopes: host_cancel={host_cancel}; 24 cycles; records=(1,0,0); live heap={baseline}");
    }
}

/// The next existing FIFO waiter progresses when a woken predecessor is cancelled.
#[test]
fn ownership_queue_fifo_reservation_cancel() {
    for (fixture, put) in [
        (include_str!("../../../../tools/coroutines/queues/fifo.py"), true),
        (include_str!("../../../../tools/coroutines/queues/waiters.py"), false),
    ] {
        let mut repl = MontyRepl::new("queues.py", ResourceTracker::default(), CompileOptions::default());
        for source in [
            include_str!("../../../../tools/coroutines/concurrent_io/wrappers.py"),
            include_str!("../../../../tools/coroutines/concurrent_io/scope.py"),
        ] {
            repl.feed_run(source, vec![], PrintWriter::Disabled).unwrap();
        }
        let mut progress = repl
            .feed_start(
                fixture,
                vec![("exchange".into(), MontyObject::function("exchange", None))],
                PrintWriter::Disabled,
            )
            .unwrap();
        while let ReplProgress::FunctionCall(call) = progress {
            progress = call.resume_pending(PrintWriter::Disabled).unwrap();
        }
        let ReplResolveFutures {
            mut repl,
            mut executor,
            vm_state,
            ..
        } = progress.into_resolve_futures().unwrap();
        HeapReader::with(&mut repl.heap, &mut executor, |reader, executor| {
            let mut vm = VM::restore(
                vm_state,
                &mut executor.tables,
                &executor.program,
                reader,
                PrintWriter::Disabled,
            );
            vm.ownership_probe_queue_waiter(put);
            reclaim_vm_state(&mut repl.globals, &mut repl.cwd, &mut repl.random, &mut vm);
        });
    }
}
