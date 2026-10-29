//! Private carrier checks without adding a public test API.
use monty_types::{PrintWriter, ResourceLimits, ResourceTracker};

use super::*;
use crate::{
    args::KwargsValues,
    asyncio::{CallId, ExternalFuture, ExternalFutureState, GatherFuture, GatherState, TaskId},
    bytecode::vm::scheduler::Scheduler,
    defer_drop_mut,
    heap::{Heap, HeapData},
    intern::Interns,
    name_map::NameMap,
    object_bridge::{ExportError, GraphExporter},
    run::{Program, SessionTables},
    types::List,
};

/// Measure private transaction/cache owners before VM or heap teardown.
#[test]
fn ownership_export_poison_depth_and_cached_error_cycles() {
    let mut heap = Heap::new(
        32,
        ResourceTracker::new(ResourceLimits::default().max_recursion_depth(4)),
    );
    let tables = SessionTables {
        interns: Interns::new(""),
        global_names: NameMap::new(),
    };
    let program = Program::for_tests();
    HeapReader::with(&mut heap, &mut (program, tables), |heap, (program, tables)| {
        let mut vm = VM::new(vec![], tables, program, heap, PrintWriter::Stdout);
        let baseline = vm.heap.entry_count();
        let token_id = vm.heap.allocate(HeapData::List(List::new(vec![])));
        let token = Value::Ref(token_id);
        let argument = token.clone_with_heap(vm.heap);
        let object = ExcType::ValueError.call(&mut vm, ArgValues::One(argument)).unwrap();
        let Value::Ref(object_id) = &object else { unreachable!() };
        let error = vm.make_exception(&object, false);
        assert_eq!(vm.heap.get_refcount(token_id), 2);
        {
            let mut transaction = DropGuard::new(GraphExporter::new(), &mut vm);
            let (exporter, vm) = transaction.as_parts_mut();
            assert!(exporter.push(&token, vm).is_ok());
            assert_eq!(vm.heap.get_refcount(token_id), 3);
            assert!(matches!(
                exporter.push(&object, vm),
                Err(ExportError::UnsupportedExceptionArgs)
            ));
            // A poisoned transaction cannot publish the earlier successful node.
            let (exporter, vm) = transaction.into_parts();
            assert!(matches!(
                exporter.finish(vm),
                Err(ExportError::UnsupportedExceptionArgs)
            ));
        }
        assert_eq!(vm.heap.get_refcount(token_id), 2);
        let positional = ArgValues::ArgsKargs {
            args: vec![
                token.clone_with_heap(vm.heap),
                object.clone_with_heap(vm.heap),
                token.clone_with_heap(vm.heap),
            ],
            kwargs: KwargsValues::Pairs(vec![(token.clone_with_heap(vm.heap), token.clone_with_heap(vm.heap))]),
        };
        assert!(matches!(
            positional.into_call_args(&mut vm),
            Err(ExportError::UnsupportedExceptionArgs)
        ));
        assert_eq!(vm.heap.get_refcount(token_id), 2);
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let keywords = ArgValues::ArgsKargs {
            args: vec![token.clone_with_heap(vm.heap)],
            kwargs: KwargsValues::Pairs(vec![
                (object.clone_with_heap(vm.heap), token.clone_with_heap(vm.heap)),
                (token.clone_with_heap(vm.heap), token.clone_with_heap(vm.heap)),
            ]),
        };
        assert!(matches!(
            keywords.into_call_args(&mut vm),
            Err(ExportError::UnsupportedExceptionArgs)
        ));
        assert_eq!(vm.heap.get_refcount(token_id), 2);
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let mut nested = token.clone_with_heap(vm.heap);
        for _ in 0..8 {
            nested = Value::Ref(vm.heap.allocate(HeapData::List(List::new(vec![nested]))));
        }
        {
            let this = &mut vm;
            let exporter = GraphExporter::new();
            defer_drop_mut!(exporter, this);
            assert!(matches!(
                exporter.push_owned(nested, this),
                Err(ExportError::DepthLimit)
            ));
        }
        assert_eq!(vm.heap.get_refcount(token_id), 2);
        let RunError::Exc(raise) = &error else { unreachable!() };
        let diagnostic = RunError::UncatchableExc(raise.clone_with_heap(vm.heap)).into_python_exception(
            vm.interns,
            |_| Some(""),
            vm.heap,
        );
        assert_eq!(diagnostic.exc_type(), ExcType::ValueError);
        assert_eq!(diagnostic.message(), None);
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let frozen = match &error {
            RunError::Exc(error) => CaughtException::Frozen {
                value: object.clone_with_heap(vm.heap),
                error: Box::new(error.clone_with_heap(vm.heap)),
            },
            _ => unreachable!(),
        };
        assert_eq!(vm.heap.get_refcount(*object_id), 4);
        frozen.drop_with(&mut vm);
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let mut scheduler = Scheduler::new();
        scheduler.block_current_on(token_id, vm.heap);
        scheduler.make_ready(TaskId::default(), Err(error.clone_with_heap(vm.heap)), vm.heap);
        scheduler.cleanup(vm.heap);
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let mut typed_future = ExternalFuture {
            call_id: CallId::new(4),
            state: ExternalFutureState::Failed(error.clone_with_heap(vm.heap)),
            sleep_result: None,
            child: None,
            queue: None,
            queue_op: None,
        };
        let mut ids = vec![];
        typed_future.py_dec_ref_ids(&mut ids);
        drop(typed_future);
        assert_eq!(ids, vec![*object_id]);
        for id in ids {
            vm.heap.dec_ref(id);
        }
        let mut typed_gather = GatherFuture {
            items: vec![],
            state: GatherState::Failed(error.clone_with_heap(vm.heap)),
        };
        let mut ids = vec![];
        typed_gather.py_dec_ref_ids(&mut ids);
        drop(typed_gather);
        assert_eq!(ids, vec![*object_id]);
        for id in ids {
            vm.heap.dec_ref(id);
        }
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        vm.fail_future(999, error.clone_with_heap(vm.heap));
        assert_eq!(
            vm.heap.get_refcount(*object_id),
            2,
            "unknown settlement drops its error"
        );
        let external_id = vm
            .heap
            .allocate(HeapData::ExternalFuture(Box::new(ExternalFuture::new_pending(
                CallId::new(0),
                None,
            ))));
        let external = Value::Ref(external_id);
        vm.scheduler.add_pending_external(CallId::new(0), external_id, vm.heap);
        vm.fail_future(0, error.clone_with_heap(vm.heap));
        assert_eq!(
            vm.heap.get_refcount(*object_id),
            3,
            "cache owns one error; absent awaiter owns none"
        );
        let gather = Value::Ref(vm.heap.allocate(HeapData::GatherFuture(Box::new(GatherFuture {
            items: vec![],
            state: GatherState::Failed(error.clone_with_heap(vm.heap)),
        }))));
        for cached in [&external, &gather] {
            for _ in 0..3 {
                vm.push(cached.clone_with_heap(vm.heap));
                let Err(result) = vm.exec_get_awaitable() else {
                    panic!("expected cached failure")
                };
                let RunError::Exc(raise) = &result else { unreachable!() };
                assert!(matches!(&raise.object, Some(Value::Ref(id)) if *id == *object_id));
                result.drop_with(&mut vm);
            }
        }
        // Old snapshots can carry metadata-only Failed states. Normalize the
        // stored cache once, rather than reconstructing on each replay.
        let legacy_external = Value::Ref(vm.heap.allocate(HeapData::ExternalFuture(Box::new(ExternalFuture {
            call_id: CallId::new(12),
            sleep_result: None,
            child: None,
            queue: None,
            queue_op: None,
            state: ExternalFutureState::Failed(ExcType::value_error("legacy")),
        }))));
        let legacy_gather = Value::Ref(vm.heap.allocate(HeapData::GatherFuture(Box::new(GatherFuture {
            items: vec![],
            state: GatherState::Failed(ExcType::value_error("legacy")),
        }))));
        for cached in [&legacy_external, &legacy_gather] {
            let mut original = None;
            for _ in 0..3 {
                vm.push(cached.clone_with_heap(vm.heap));
                let Err(result) = vm.exec_get_awaitable() else {
                    panic!("legacy cache failure")
                };
                let RunError::Exc(raise) = &result else { unreachable!() };
                let Some(Value::Ref(id)) = &raise.object else {
                    panic!("cache identity")
                };
                if let Some(original) = original {
                    assert_eq!(*id, original);
                } else {
                    original = Some(*id);
                }
                result.drop_with(&mut vm);
            }
        }
        legacy_external.drop_with(&mut vm);
        legacy_gather.drop_with(&mut vm);
        let HeapReadOutput::List(list) = vm.heap.read(token_id) else {
            unreachable!()
        };
        list.append(&mut vm, external).unwrap();
        list.append(&mut vm, gather).unwrap();
        drop(list);
        error.drop_with(&mut vm);
        object.drop_with(&mut vm);
        token.drop_with(&mut vm);
        vm.__force_gc_for_tests();
        assert_eq!(
            vm.heap.entry_count(),
            baseline,
            "both failed caches, error, tuple and token cycle reclaimed before teardown"
        );
    });
}

/// Retained terminal export metadata cannot own guest identity, even when the
/// original error does. Cloning the latch preserves classification and location.
#[test]
fn ownership_terminal_export_metadata_releases_identity() {
    let mut heap = Heap::new(32, ResourceTracker::default());
    let tables = SessionTables {
        interns: Interns::new(""),
        global_names: NameMap::new(),
    };
    let program = Program::for_tests();
    HeapReader::with(&mut heap, &mut (program, tables), |heap, (program, tables)| {
        let mut vm = VM::new(vec![], tables, program, heap, PrintWriter::Stdout);
        let baseline = vm.heap.entry_count();
        let token_id = vm.heap.allocate(HeapData::List(List::new(vec![])));
        let token = Value::Ref(token_id);
        let argument = token.clone_with_heap(vm.heap);
        let object = ExcType::ValueError.call(&mut vm, ArgValues::One(argument)).unwrap();
        let Value::Ref(object_id) = &object else { unreachable!() };
        let RunError::Exc(error) = vm.make_exception(&object, false) else {
            unreachable!()
        };
        assert_eq!(vm.heap.get_refcount(*object_id), 2);
        let original_frame = error.frame.clone();
        let terminal = ExportError::terminal(error, &mut vm);
        assert_eq!(vm.heap.get_refcount(*object_id), 1);
        for diagnostic in [terminal.clone(), terminal] {
            let RunError::UncatchableExc(error) = diagnostic.into_run_error(&vm) else {
                panic!("terminal classification")
            };
            assert!(error.object.is_none());
            assert_eq!(error.exc.exc_type(), ExcType::ValueError);
            assert_eq!(error.frame.is_some(), original_frame.is_some());
            error.drop_with(&mut vm);
        }
        object.drop_with(&mut vm);
        assert_eq!(vm.heap.get_refcount(token_id), 1);
        token.drop_with(&mut vm);
        assert_eq!(vm.heap.entry_count(), baseline);
    });
}

#[path = "guest_queue.rs"]
mod guest_queue;
