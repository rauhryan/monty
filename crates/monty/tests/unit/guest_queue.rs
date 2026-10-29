//! Queue delivery seam and live ownership checks, including cyclic queue edges.
use super::*;
use crate::asyncio::Awaiter;

/// Exercise real queue registration/wake/activation with no external race hook.
#[test]
fn ownership_queue_delivery_and_retention() {
    let mut heap = Heap::new(32, ResourceTracker::default());
    let tables = SessionTables {
        interns: Interns::new(""),
        global_names: NameMap::new(),
    };
    let program = Program::for_tests();
    HeapReader::with(&mut heap, &mut (program, tables), |heap, (program, tables)| {
        let mut vm = VM::new(vec![], tables, program, heap, PrintWriter::Disabled);
        let baseline = vm.heap.entry_count();
        for capacity in [
            Value::Int(0),
            Value::Int(-1),
            Value::Int(4_294_967_296),
            Value::Bool(true),
        ] {
            vm.new_guest_queue(&capacity).unwrap_err().drop_with(&mut vm);
        }
        let maximum = vm.new_guest_queue(&Value::Int(4_294_967_295)).unwrap();
        maximum.drop_with(&mut vm);
        let task = TaskId::default();
        for _ in 0..32 {
            let queue = vm.new_guest_queue(&Value::Int(1)).unwrap();
            let Value::Ref(qid) = &queue else { unreachable!() };
            // A self-referencing value also points back to its owning queue.
            let cycle = vm.heap.allocate(HeapData::List(List::new(vec![])));
            vm.heap.inc_ref(cycle);
            let qvalue = queue.clone_with_heap(vm.heap);
            let HeapReadOutput::List(list) = vm.heap.read(cycle) else {
                unreachable!()
            };
            list.append(&mut vm, Value::Ref(cycle)).unwrap();
            list.append(&mut vm, qvalue).unwrap();
            drop(list);
            let value = Value::Ref(cycle);
            // A reserved put is not yet in queue storage. Cancellation revokes it.
            let put = vm.new_queue_operation(&queue, Some(&value)).unwrap();
            let Value::Ref(id) = &put else { unreachable!() };
            vm.prepare_queue_operation(*id, &Awaiter::Task(task)).unwrap();
            vm.scheduler.block_current_on(*id, vm.heap);
            vm.wake_queue_operation(*id);
            assert_eq!(queue_counts(&vm, *qid), (0, 0, 1, true));
            vm.cancel_child_task(task, "guest");
            vm.scheduler.remove_from_ready_queue(task);
            vm.activate_task(task).unwrap_err().drop_with(&mut vm);
            assert_eq!(queue_counts(&vm, *qid), (0, 0, 0, false));
            assert!(vm.scheduler.get_task_mut(task).pending_queue.is_none());
            vm.scheduler.get_task_mut(task).native_cancel = None;
            vm.scheduler
                .get_task_mut(task)
                .child_cancel_signal
                .take()
                .drop_with(vm.heap);
            put.drop_with(&mut vm);
            // Successful delivery commits exactly once.
            let put = vm.new_queue_operation(&queue, Some(&value)).unwrap();
            let Value::Ref(id) = &put else { unreachable!() };
            vm.prepare_queue_operation(*id, &Awaiter::Task(task)).unwrap();
            vm.scheduler.block_current_on(*id, vm.heap);
            vm.wake_queue_operation(*id);
            vm.scheduler.remove_from_ready_queue(task);
            vm.activate_task(task).unwrap();
            vm.pop().drop_with(&mut vm);
            put.drop_with(&mut vm);
            assert_eq!(queue_counts(&vm, *qid), (1, 0, 0, false));
            // A reserved get leaves the item in storage until delivery.
            let get = vm.new_queue_operation(&queue, None).unwrap();
            let Value::Ref(id) = &get else { unreachable!() };
            vm.prepare_queue_operation(*id, &Awaiter::Task(task)).unwrap();
            vm.scheduler.block_current_on(*id, vm.heap);
            vm.wake_queue_operation(*id);
            assert_eq!(queue_counts(&vm, *qid), (1, 1, 0, true));
            vm.cancel_child_task(task, "guest");
            vm.scheduler.remove_from_ready_queue(task);
            vm.activate_task(task).unwrap_err().drop_with(&mut vm);
            assert_eq!(queue_counts(&vm, *qid), (1, 0, 0, false));
            vm.scheduler.get_task_mut(task).native_cancel = None;
            vm.scheduler
                .get_task_mut(task)
                .child_cancel_signal
                .take()
                .drop_with(vm.heap);
            get.drop_with(&mut vm);
            let get = vm.new_queue_operation(&queue, None).unwrap();
            let Value::Ref(id) = &get else { unreachable!() };
            vm.prepare_queue_operation(*id, &Awaiter::Task(task)).unwrap();
            vm.scheduler.block_current_on(*id, vm.heap);
            vm.wake_queue_operation(*id);
            vm.scheduler.remove_from_ready_queue(task);
            vm.activate_task(task).unwrap();
            assert!(matches!(vm.stack.last(), Some(Value::Ref(id)) if *id == cycle));
            vm.pop().drop_with(&mut vm);
            get.drop_with(&mut vm);
            assert_eq!(queue_counts(&vm, *qid), (0, 0, 0, false));
            // Abandon a waiter while its queue remains reachable; no stale task route.
            let abandoned = vm.new_queue_operation(&queue, None).unwrap();
            let Value::Ref(id) = &abandoned else { unreachable!() };
            vm.prepare_queue_operation(*id, &Awaiter::Task(task)).unwrap();
            vm.scheduler.block_current_on(*id, vm.heap);
            vm.scheduler.cleanup(vm.heap);
            assert_eq!(queue_counts(&vm, *qid), (0, 0, 0, false));
            vm.scheduler = Scheduler::new();
            abandoned.drop_with(&mut vm);
            value.drop_with(&mut vm);
            queue.drop_with(&mut vm);
            vm.heap.collect_cycles();
            assert_eq!(
                vm.heap.entry_count(),
                baseline,
                "no retained queue/waiter/value records before teardown"
            );
        }
        eprintln!("queue: 32 put/get wake-cancel-delivery lifecycles; live heap returns to {baseline}");
    });
}

/// Count actual heap queue records, not a candidate-provided observation flag.
fn queue_counts(vm: &VM<'_>, id: HeapId) -> (usize, usize, usize, bool) {
    let HeapData::ExternalFuture(f) = vm.heap.get(id) else {
        unreachable!()
    };
    let q = f.queue.as_ref().unwrap();
    (q.values.len(), q.getters.len(), q.putters.len(), q.reserved.is_some())
}
