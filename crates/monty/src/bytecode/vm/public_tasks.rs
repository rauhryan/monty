//! Public asyncio Tasks: scheduler-owned execution and handle-owned cached outcomes.
use std::mem;

use super::{AwaitResult, VM};
use crate::{
    args::ArgValues,
    asyncio::{Awaiter, CallId, CoroutineState, ExternalFuture, ExternalFutureState, PublicTask, TaskId},
    bytecode::CallResult,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    modules::{ModuleFunctions, asyncio::AsyncioFunctions},
    types::BoundMethod,
    value::{EitherStr, Value},
};

impl VM<'_> {
    /// Admit independently of experimental scopes. The live scheduler owns the
    /// outcome root even when the last guest handle is discarded.
    pub(crate) fn spawn_public_task(&mut self, value: &Value) -> RunResult<Value> {
        if self.native_cancel_reason().is_some()
            || self
                .scheduler
                .current_task()
                .is_some_and(|task| task.native_cancel.is_some())
        {
            return Err(ExcType::value_error("task admission is closed"));
        }
        let Value::Ref(coroutine) = value else {
            return Err(ExcType::type_error("create_task requires a new coroutine"));
        };
        if !matches!(self.heap.get(*coroutine), HeapData::Coroutine(c) if c.state == CoroutineState::New) {
            return Err(ExcType::type_error("create_task requires a new coroutine"));
        }
        let task = self
            .scheduler
            .spawn(self.heap, *coroutine, None)
            .ok_or_else(ExcType::cannot_reuse_already_awaited_coroutine)?;
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.public_task = Some(PublicTask {
            task: Some(task),
            cancelled: false,
            waiters: Vec::new(),
        });
        let id = self.heap.allocate(HeapData::ExternalFuture(Box::new(future)));
        self.heap.inc_ref(id);
        self.scheduler.get_task_mut(task).child_result = Some(id);
        // A task's creator may exit before it. Remaining gather obligations
        // transfer to the explicit run owner, or the legacy execution root.
        let owner = self.scheduler.get_task_mut(task).run_owner.unwrap_or_default();
        self.scheduler.get_task_mut(task).lifetime_parent = Some(owner);
        self.scheduler.get_task_mut(owner).gather_children.push(task);
        Ok(Value::Ref(id))
    }

    pub(crate) fn public_task_method(&mut self, id: HeapId, attr: &EitherStr) -> Option<CallResult> {
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            return None;
        };
        if future
            .queue
            .as_ref()
            .is_some_and(|queue| queue.public_maxsize.is_some())
        {
            return self.public_queue_method(id, attr);
        }
        if future.task_group.is_some() {
            return self.task_group_method(id, attr);
        }
        future.public_task.as_ref()?;
        let function = match attr.as_str(self.interns) {
            "done" => AsyncioFunctions::TaskDone,
            "cancelled" => AsyncioFunctions::TaskCancelled,
            "cancel" => AsyncioFunctions::TaskCancel,
            "result" => AsyncioFunctions::TaskResult,
            "exception" => AsyncioFunctions::TaskException,
            _ => return None,
        };
        self.heap.inc_ref(id);
        let method = BoundMethod {
            instance: Value::Ref(id),
            func: Value::ModuleFunction(ModuleFunctions::Asyncio(function)),
        };
        Some(CallResult::Value(Value::Ref(
            self.heap.allocate(HeapData::BoundMethod(method)),
        )))
    }

    pub(crate) fn call_public_task(&mut self, id: HeapId, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let name = attr.as_str(self.interns).to_owned();
        if !matches!(name.as_str(), "done" | "cancelled" | "cancel" | "result" | "exception") {
            args.drop_with(self);
            return Err(ExcType::attribute_error("Task", &name));
        }
        args.check_zero_args(&name, self.heap)?;
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let public = future.get(self.heap).public_task.as_ref().expect("public task");
        let value = match name.as_str() {
            "done" => Value::Bool(public.task.is_none()),
            "cancelled" => Value::Bool(public.cancelled),
            "cancel" => {
                let task = public.task;
                drop(future);
                if let Some(task) = task {
                    self.cancel_public_task(task);
                }
                return Ok(CallResult::Value(Value::Bool(task.is_some())));
            }
            _ => match &future.get(self.heap).state {
                ExternalFutureState::Pending { .. } => {
                    return Err(SimpleException::new_msg(ExcType::RuntimeError, "Result is not ready.").into());
                }
                ExternalFutureState::Resolved(value) => {
                    if name == "exception" {
                        Value::None
                    } else {
                        value.clone_with_heap(self.heap)
                    }
                }
                ExternalFutureState::Failed(error) => {
                    if name == "result" || public.cancelled {
                        return Err(error.clone_with_heap(self.heap));
                    }
                    let RunError::Exc(error) = error else {
                        return Err(error.clone_with_heap(self.heap));
                    };
                    error
                        .object
                        .as_ref()
                        .expect("materialized task error")
                        .clone_with_heap(self.heap)
                }
            },
        };
        Ok(CallResult::Value(value))
    }

    /// Follow pending Task joins without turning them into lifetime ownership.
    /// Iterate rather than recurse: mutual waits must not loop or overflow.
    pub(super) fn cancel_public_task(&mut self, root: TaskId) {
        self.cancel_public_task_reason(root, "guest");
    }

    pub(super) fn cancel_public_task_reason(&mut self, root: TaskId, reason: &str) {
        let mut pending = vec![root];
        let mut signals = Vec::new();
        // Register the entire forwarding graph before signalling an owned
        // subtree. Otherwise an intermediate coroutine could detach its join
        // before we discover that its awaited public Task also needs cancellation.
        while let Some(task) = pending.pop() {
            if signals.iter().any(|(visited, _)| *visited == task) || self.scheduler.task(task).is_none() {
                continue;
            }
            let record = self.scheduler.task(task).expect("live task");
            let protected = record.native_protected || record.gather_protected;
            let mut gathered = false;
            let mut targets = Vec::new();
            if !protected && let Some(wait) = self.scheduler.blocked_on(task) {
                gathered = matches!(self.heap.get(wait), HeapData::GatherFuture(_));
                let mut waits = vec![wait];
                while let Some(wait) = waits.pop() {
                    match self.heap.get(wait) {
                        HeapData::ExternalFuture(future) => {
                            if let Some(target) = future.public_task.as_ref().and_then(|task| task.task) {
                                targets.push(target);
                            }
                        }
                        HeapData::GatherFuture(gather) => waits.extend(gather.items.iter().copied()),
                        _ => {}
                    }
                }
            }
            if !targets.is_empty() {
                self.scheduler
                    .public_gather_waits
                    .entry(task)
                    .or_default()
                    .extend(targets.iter().copied());
            }
            pending.extend(targets);
            pending.extend(
                self.scheduler
                    .owned_subtree(task, self.heap)
                    .into_iter()
                    .filter(|child| *child != task),
            );
            signals.push((task, gathered));
        }
        for (task, gathered) in signals {
            self.cancel_child_task(task, reason);
            if (gathered || self.scheduler.blocked_on(task).is_none())
                && (!self.scheduler.get_task_mut(task).gather_children.is_empty()
                    || self
                        .scheduler
                        .public_gather_waits
                        .get(&task)
                        .is_some_and(|waits| !waits.is_empty()))
            {
                self.scheduler.get_task_mut(task).pending_gather_error =
                    super::scheduler::GatherErrorDelivery::Draining;
                self.scheduler.remove_from_ready_queue(task);
            }
        }
    }

    pub(super) fn prepare_public_join(&self, id: HeapId, awaiter: &Awaiter) -> RunResult<()> {
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            unreachable!()
        };
        let public = future.public_task.as_ref().expect("public task");
        if matches!(awaiter, Awaiter::Task(task) if public.task == Some(*task)) {
            return Err(SimpleException::new_msg(ExcType::RuntimeError, "Task cannot await on itself").into());
        }
        Ok(())
    }

    pub(super) fn is_public_task(&self, task: TaskId) -> bool {
        self.scheduler
            .task(task)
            .and_then(|task| task.child_result)
            .is_some_and(
                |id| matches!(self.heap.get(id), HeapData::ExternalFuture(future) if future.public_task.is_some()),
            )
    }

    pub(super) fn complete_public_task(&mut self, mut result: RunResult<Value>) -> RunResult<AwaitResult> {
        let task = self.scheduler.current_task_id().expect("public task");
        let record = self.scheduler.get_task_mut(task);
        let group = record.group_parent;
        let id = record.child_result.take().expect("outcome root");
        let cancelled = matches!((&result, &record.child_cancel_signal),
            (Err(RunError::Exc(error)), Some(Value::Ref(signal)))
                if matches!(error.object, Some(Value::Ref(id)) if id == *signal));
        let coroutine = record.coroutine_id.expect("public coroutine");
        let HeapReadOutput::Coroutine(coro) = self.heap.read(coroutine) else {
            unreachable!()
        };
        coro.get_mut(self.heap).state = CoroutineState::Completed;
        drop(coro);
        if let Err(error) = &mut result {
            error.materialize_object(self.heap);
        }
        let state = match &result {
            Ok(value) => ExternalFutureState::Resolved(value.clone_with_heap(self.heap)),
            Err(error) => ExternalFutureState::Failed(error.clone_with_heap(self.heap)),
        };
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let public = future.get_mut(self.heap).public_task.as_mut().expect("public task");
        public.task = None;
        public.cancelled = cancelled;
        let waiters = mem::take(&mut public.waiters);
        future.get_mut(self.heap).state = state;
        drop(future);
        let parent = self.scheduler.transfer_gather_lifetime(task);
        self.cleanup_current_task();
        self.scheduler.cancel_task(task, self.heap);
        // Observe group failure while the parent's await graph is still
        // attached. A first-error gather delivery must not erase the body
        // operands which structured interruption needs to cancel and drain.
        let failed_group = !cancelled && result.is_err();
        if failed_group && let Some(group) = group {
            self.task_group_child_completed(group, true);
        }
        // Cache remains owned by handles; every pending join gets its own clone.
        for waiter in waiters {
            if let Awaiter::Task(waiting) = &waiter
                && self.scheduler.blocked_on(*waiting) == Some(id)
            {
                let record = self.scheduler.get_task_mut(*waiting);
                if !record.native_protected
                    && !record.gather_protected
                    && let Some(cancel) = &mut record.native_cancel
                    && !cancel.delivered
                {
                    cancel.delivered = true;
                    record.child_cancel_signal.take().drop_with(self.heap);
                    if cancelled && let Err(RunError::Exc(error)) = &result {
                        record.child_cancel_signal =
                            error.object.as_ref().map(|value| value.clone_with_heap(self.heap));
                    }
                }
            }
            match &result {
                Ok(value) => {
                    self.deliver_awaiter_success(waiter, value.clone_with_heap(self.heap));
                }
                Err(error) => {
                    self.deliver_awaiter_failure(waiter, error.clone_with_heap(self.heap));
                }
            }
        }
        if !failed_group && let Some(group) = group {
            self.task_group_child_completed(group, false);
        }
        match result {
            Ok(value) => value.drop_with(self),
            Err(error) => error.drop_with(self),
        }
        Value::Ref(id).drop_with(self);
        let mut owners: Vec<_> = self
            .scheduler
            .public_gather_waits
            .iter_mut()
            .filter_map(|(owner, waits)| {
                let had = waits.contains(&task);
                waits.retain(|wait| *wait != task);
                had.then_some(*owner)
            })
            .collect();
        // Hash-map iteration is not stable across fresh restores. Release
        // independent barriers in creation order, retaining deterministic traces.
        owners.sort_unstable();
        self.scheduler.public_gather_waits.retain(|_, waits| !waits.is_empty());
        for owner in owners {
            self.finish_child_drain(owner);
        }
        if let Some(parent) = parent {
            self.finish_child_drain(parent);
        }
        self.finish_child_drain(TaskId::default());
        if self.resume_after_task_exit(None)? {
            Ok(AwaitResult::FramePushed)
        } else {
            Ok(AwaitResult::Yield(self.scheduler.pending_call_ids()))
        }
    }
}
