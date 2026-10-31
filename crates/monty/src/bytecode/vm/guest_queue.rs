//! Bounded guest queues. A wake reserves permission; only task activation commits.
use std::{collections::VecDeque, mem::size_of};

use super::{CallResult, VM, scheduler::Scheduler};
use crate::{
    args::ArgValues,
    asyncio::{Awaiter, CallId, ExternalFuture, ExternalFutureState, GuestQueue, QueueOperation, TaskId},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput, HeapReader},
    modules::{
        ModuleFunctions,
        asyncio::{self as asyncio_module, AsyncioFunctions},
    },
    types::BoundMethod,
    value::{EitherStr, Value},
};

impl VM<'_> {
    /// Construct an opaque token, with no eager allocation proportional to capacity.
    pub(crate) fn new_guest_queue(&mut self, capacity: &Value) -> RunResult<Value> {
        let Value::Int(capacity) = capacity else {
            return Err(ExcType::type_error("queue capacity must be a positive integer"));
        };
        let capacity = u32::try_from(*capacity)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| ExcType::value_error("queue capacity must be between 1 and 4294967295"))?;
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.queue = Some(GuestQueue {
            capacity: capacity as usize,
            public_maxsize: None,
            values: VecDeque::new(),
            getters: VecDeque::new(),
            putters: VecDeque::new(),
            reserved: None,
        });
        Ok(Value::Ref(
            self.heap.allocate(HeapData::ExternalFuture(Box::new(future))),
        ))
    }

    /// Public construction uses the same reservation protocol, with zero as unbounded.
    pub(crate) fn new_public_queue(&mut self, maxsize: &Value) -> RunResult<Value> {
        let size = match maxsize {
            Value::Int(n) => *n,
            Value::Bool(b) => i64::from(*b),
            _ => return Err(ExcType::type_error("Queue maxsize must be an integer")),
        };
        let capacity = if size <= 0 {
            0
        } else {
            u32::try_from(size).map_err(|_| ExcType::value_error("Queue maxsize must be at most 4294967295"))?
        };
        let value = self.new_guest_queue(&Value::Int(1))?;
        let Value::Ref(id) = &value else { unreachable!() };
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(*id) else {
            unreachable!()
        };
        let queue = future.get_mut(self.heap).queue.as_mut().expect("queue");
        queue.capacity = capacity as usize;
        queue.public_maxsize = Some(size);
        Ok(value)
    }

    /// Create a lazy, single-await operation. No queue mutation before await.
    pub(crate) fn new_queue_operation(&mut self, queue: &Value, put: Option<&Value>) -> RunResult<Value> {
        let Value::Ref(id) = queue else {
            return Err(ExcType::type_error("expected a native queue"));
        };
        if !matches!(self.heap.get(*id), HeapData::ExternalFuture(f) if f.queue.is_some()) {
            return Err(ExcType::type_error("expected a native queue"));
        }
        self.heap.inc_ref(*id);
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.queue_op = Some(QueueOperation {
            queue: Some(*id),
            put: put.map(|v| v.clone_with_heap(self.heap)),
            started: false,
            task: None,
        });
        Ok(Value::Ref(
            self.heap.allocate(HeapData::ExternalFuture(Box::new(future))),
        ))
    }

    /// Admit direct task awaits only; coroutine wrappers can participate in gather.
    pub(super) fn prepare_queue_operation(&mut self, id: HeapId, awaiter: &Awaiter) -> RunResult<()> {
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        if future.get(self.heap).queue.is_some() {
            return Err(ExcType::type_error("queue tokens are not awaitable"));
        }
        let Some(op) = &future.get(self.heap).queue_op else {
            return Ok(());
        };
        if op.started {
            return Err(ExcType::value_error("queue operations may only be awaited once"));
        }
        let Awaiter::Task(task) = awaiter else {
            return Err(ExcType::type_error(
                "wrap queue operations in a coroutine before gather",
            ));
        };
        let queue = op.queue.expect("new operation owns queue");
        let put = op.put.is_some();
        let HeapReadOutput::ExternalFuture(q) = self.heap.read(queue) else {
            unreachable!()
        };
        let state = q.get(self.heap).queue.as_ref().expect("queue token");
        let waiters = if put { &state.putters } else { &state.getters };
        self.heap
            .tracker
            .check_growth(waiters.len(), waiters.capacity(), size_of::<HeapId>())?;
        self.heap.inc_ref(id); // queue waiter root
        let state = q.get_mut(self.heap).queue.as_mut().expect("queue token");
        if put {
            state.putters.push_back(id);
        } else {
            state.getters.push_back(id);
        }
        let op = future.get_mut(self.heap).queue_op.as_mut().expect("operation");
        op.started = true;
        op.task = Some(*task);
        self.heap.inc_ref(id); // survives wake until task activation
        self.scheduler.get_task_mut(*task).pending_queue = Some(id);
        Ok(())
    }

    /// Called after the task becomes blocked; never wake a still-running await.
    pub(super) fn wake_queue_operation(&mut self, id: HeapId) {
        let queue = match self.heap.get(id) {
            HeapData::ExternalFuture(f) => f.queue_op.as_ref().and_then(|op| op.queue),
            _ => None,
        };
        if let Some(queue) = queue {
            self.pump_queue(queue);
        }
    }

    /// Share wake bookkeeping with physical scheduler disposal.
    fn pump_queue(&mut self, queue: HeapId) {
        self.scheduler.pump_queue(queue, self.heap);
    }

    /// Revoke a waiter or ready reservation before injecting cancellation.
    pub(super) fn cancel_queue_operation(&mut self, task: TaskId) {
        if let Some(id) = self.scheduler.get_task_mut(task).pending_queue.take() {
            self.finish_queue_operation(id, false);
            self.heap.dec_ref(id);
        }
    }

    /// Deliver directly to the activated task, after its cancellation checkpoint.
    pub(super) fn consume_queue_delivery(&mut self, task: TaskId) -> RunResult<()> {
        if let Some(id) = self.scheduler.get_task_mut(task).pending_queue {
            let HeapData::ExternalFuture(f) = self.heap.get(id) else {
                unreachable!()
            };
            let op = f.queue_op.as_ref().expect("operation");
            if op.put.is_some() {
                let HeapData::ExternalFuture(q) = self.heap.get(op.queue.expect("queue root")) else {
                    unreachable!()
                };
                let q = q.queue.as_ref().expect("queue");
                if let Err(error) =
                    self.heap
                        .tracker
                        .check_growth(q.values.len(), q.values.capacity(), size_of::<Value>())
                {
                    self.cancel_queue_operation(task);
                    return Err(error.into());
                }
            }
            self.scheduler.get_task_mut(task).pending_queue = None;
            let value = self.finish_queue_operation(id, true);
            self.stack.push(value);
            self.heap.dec_ref(id);
        }
        Ok(())
    }

    /// Commit or revoke, then offer the next FIFO waiter its reservation.
    fn finish_queue_operation(&mut self, id: HeapId, commit: bool) -> Value {
        let (value, queue) = finish_queue_operation(id, commit, self.heap);
        self.pump_queue(queue);
        self.heap.dec_ref(queue);
        value
    }
}

impl Scheduler {
    /// Serialize deliveries, favoring an eligible getter. Each side is FIFO.
    pub(super) fn pump_queue(&mut self, queue: HeapId, heap: &mut HeapReader<'_>) {
        let HeapReadOutput::ExternalFuture(q) = heap.read(queue) else {
            unreachable!()
        };
        let state = q.get_mut(heap).queue.as_mut().expect("queue");
        if state.reserved.is_some() {
            return;
        }
        let next = if !state.values.is_empty() && !state.getters.is_empty() {
            state.getters.front().copied()
        } else if state.capacity == 0 || state.values.len() < state.capacity {
            state.putters.front().copied()
        } else {
            None
        };
        state.reserved = next;
        if let Some(id) = next {
            let HeapData::ExternalFuture(f) = heap.get(id) else {
                unreachable!()
            };
            let task = f.queue_op.as_ref().expect("operation").task.expect("waiting task");
            drop(q);
            self.make_ready(task, Ok(()), heap);
        }
    }
}

/// Remove all waiter roots on either path; a cancelled put never enters storage.
pub(super) fn finish_queue_operation(id: HeapId, commit: bool, heap: &mut HeapReader<'_>) -> (Value, HeapId) {
    let HeapReadOutput::ExternalFuture(f) = heap.read(id) else {
        unreachable!()
    };
    let op = f.get_mut(heap).queue_op.as_mut().expect("operation");
    let queue = op.queue.take().expect("live operation owns queue");
    let put = op.put.take();
    op.task = None;
    f.get_mut(heap).state = ExternalFutureState::Resolved(Value::None);
    drop(f);
    let HeapReadOutput::ExternalFuture(q) = heap.read(queue) else {
        unreachable!()
    };
    let state = q.get_mut(heap).queue.as_mut().expect("queue");
    if commit {
        assert_eq!(state.reserved, Some(id));
    }
    if state.reserved == Some(id) {
        state.reserved = None;
    }
    let waiters = if put.is_some() {
        &mut state.putters
    } else {
        &mut state.getters
    };
    let index = waiters.iter().position(|v| *v == id).expect("registered waiter");
    waiters.remove(index);
    let (result, abandoned) = if commit {
        if let Some(value) = put {
            state.values.push_back(value);
            (Value::None, None)
        } else {
            (state.values.pop_front().expect("reserved value"), None)
        }
    } else {
        (Value::None, put)
    };
    drop(q);
    abandoned.drop_with(heap);
    heap.dec_ref(id);
    (result, queue)
}

#[cfg(test)]
impl VM<'_> {
    /// Exercise FIFO cancellation at the internal delivery seam of real blocked tasks.
    pub(crate) fn ownership_probe_queue_waiter(&mut self, put: bool) {
        let task = self
            .scheduler
            .owned_task_tree(self.heap)
            .into_iter()
            .find(|task| self.scheduler.task(*task).unwrap().pending_queue.is_some())
            .unwrap();
        let op = self.scheduler.get_task_mut(task).pending_queue.unwrap();
        let HeapData::ExternalFuture(f) = self.heap.get(op) else {
            unreachable!()
        };
        let queue = f.queue_op.as_ref().unwrap().queue.unwrap();
        let HeapReadOutput::ExternalFuture(q) = self.heap.read(queue) else {
            unreachable!()
        };
        let state = q.get_mut(self.heap).queue.as_mut().unwrap();
        let waiters = if put { &state.putters } else { &state.getters };
        assert!(waiters.len() >= 2);
        let (first, second) = (waiters[0], waiters[1]);
        let removed = if put {
            state.values.pop_front()
        } else {
            state.values.push_back(Value::Int(123));
            None
        };
        drop(q);
        removed.drop_with(self.heap);
        self.pump_queue(queue);
        let HeapData::ExternalFuture(f) = self.heap.get(first) else {
            unreachable!()
        };
        let first_task = f.queue_op.as_ref().unwrap().task.unwrap();
        let HeapData::ExternalFuture(f) = self.heap.get(second) else {
            unreachable!()
        };
        let second_task = f.queue_op.as_ref().unwrap().task.unwrap();
        self.cancel_child_task(first_task, "guest");
        self.scheduler.remove_from_ready_queue(first_task);
        self.activate_task(first_task).unwrap_err().drop_with(self);
        let HeapData::ExternalFuture(q) = self.heap.get(queue) else {
            unreachable!()
        };
        let state = q.queue.as_ref().unwrap();
        assert_eq!(state.reserved, Some(second));
        assert_eq!(state.values.len(), usize::from(!put));
        self.scheduler.remove_from_ready_queue(second_task);
        self.activate_task(second_task).unwrap();
        let delivered = self.pop();
        if !put {
            assert!(matches!(delivered, Value::Int(123)));
        }
        delivered.drop_with(self);
        let HeapData::ExternalFuture(q) = self.heap.get(queue) else {
            unreachable!()
        };
        let state = q.queue.as_ref().unwrap();
        assert_eq!(state.values.len(), usize::from(put));
        if put {
            assert_eq!(state.putters.len(), 0);
        }
        eprintln!("queue put={put}: first wake cancelled, next FIFO waiter delivered, values preserved");
    }
}

impl VM<'_> {
    /// Queue inspection reads committed storage; a reservation has not transferred ownership.
    pub(crate) fn public_queue_method(&mut self, id: HeapId, attr: &EitherStr) -> Option<CallResult> {
        let function = match attr.as_str(self.interns) {
            "maxsize" => {
                let HeapData::ExternalFuture(future) = self.heap.get(id) else {
                    unreachable!()
                };
                return Some(CallResult::Value(Value::Int(future.queue.as_ref()?.public_maxsize?)));
            }
            "put" => AsyncioFunctions::QueuePut,
            "get" => AsyncioFunctions::QueueGet,
            "qsize" => AsyncioFunctions::QueueSize,
            "empty" => AsyncioFunctions::QueueEmpty,
            "full" => AsyncioFunctions::QueueFull,
            _ => return None,
        };
        self.heap.inc_ref(id);
        Some(CallResult::Value(Value::Ref(self.heap.allocate(
            HeapData::BoundMethod(BoundMethod {
                instance: Value::Ref(id),
                func: Value::ModuleFunction(ModuleFunctions::Asyncio(function)),
            }),
        ))))
    }
}

impl VM<'_> {
    /// Direct calls and extracted bound methods share the same argument binder.
    pub(crate) fn call_public_queue(&mut self, id: HeapId, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let function = match attr.as_str(self.interns) {
            "put" => AsyncioFunctions::QueuePut,
            "get" => AsyncioFunctions::QueueGet,
            "qsize" => AsyncioFunctions::QueueSize,
            "empty" => AsyncioFunctions::QueueEmpty,
            "full" => AsyncioFunctions::QueueFull,
            name => {
                args.drop_with(self);
                return Err(ExcType::attribute_error("Queue", name));
            }
        };
        self.heap.inc_ref(id);
        asyncio_module::call(self, function, args.prepend(Value::Ref(id)))
    }
}
