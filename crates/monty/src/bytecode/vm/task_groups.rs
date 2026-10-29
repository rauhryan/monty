//! Public structured groups over scheduler-owned Tasks, never private scope aliases.
use std::mem;

use super::{CallResult, VM, scheduler::GatherErrorDelivery};
use crate::{
    args::ArgValues,
    asyncio::{Awaiter, CallId, ExternalFuture, ExternalFutureState, TaskGroup, TaskGroupState, TaskId},
    exception_private::{ExcType, ExcTypeExt, ExceptionObject, ExceptionRaise, RunError, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    modules::{ModuleFunctions, asyncio::AsyncioFunctions},
    types::{BoundMethod, allocate_tuple},
    value::{EitherStr, Value},
};

impl VM<'_> {
    /// Park the root's pending return while unfinished scheduler obligations
    /// logically cancel and drain. The authored return remains on its saved stack.
    pub(super) fn root_shutdown_return(&mut self) -> Option<RunResult<super::AwaitResult>> {
        if !self.at_run_frame() {
            return None;
        }
        let owner = self.scheduler.current_task_id().expect("run owner");
        if let Some(id) = self.scheduler.get_task_mut(owner).root_shutdown.take() {
            // The barrier wake pushed only a sentinel above the pending return.
            self.pop().drop_with(self);
            Value::Ref(id).drop_with(self);
            return None;
        }
        let children = self.scheduler.run_tasks(owner);
        if children.is_empty() && self.scheduler.run_externals(owner).is_empty() {
            return None;
        }
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.state = ExternalFutureState::Pending {
            awaiter: Some(Awaiter::Task(owner)),
        };
        let id = self.heap.allocate(HeapData::ExternalFuture(Box::new(future)));
        self.scheduler.get_task_mut(owner).root_shutdown = Some(id);
        self.current_frame.ip = self.instruction_ip;
        self.scheduler.get_task_mut(owner).native_protected = true;
        self.scheduler.block_current_on(id, self.heap);
        for child in children {
            if self.scheduler.task(child).is_some() {
                self.cancel_public_task(child);
            }
        }
        for call in self.scheduler.run_externals(owner) {
            self.fail_future(call.raw(), ExcType::value_error("guest"));
        }
        self.finish_root_shutdown(owner);
        Some(self.switch_or_yield())
    }

    pub(super) fn root_error_needs_shutdown(&self) -> bool {
        self.at_run_frame()
            && self.scheduler.current_task().is_some_and(|task| {
                task.root_shutdown.is_none()
                    && (!self.scheduler.run_tasks(task.id).is_empty()
                        || !self.scheduler.run_externals(task.id).is_empty())
            })
    }

    fn at_run_frame(&self) -> bool {
        self.scheduler.current_task().is_some_and(|task| {
            task.run_frame == Some(self.suspended_frames.len())
                    // A genuine old producer already established this barrier.
                    // Its saved continuation is authoritative; do not infer a new
                    // boundary for older states which have no barrier at all.
                    || (task.run_frame.is_none() && task.root_shutdown.is_some())
        })
    }

    pub(super) fn leave_run_frame(&mut self) {
        if self.at_run_frame() {
            let owner = self.scheduler.current_task_id().expect("run owner");
            self.scheduler.get_task_mut(owner).run_frame = None;
        }
    }

    pub(super) fn root_error_shutdown(&mut self, error: RunError) -> RunResult<()> {
        let owner = self.scheduler.current_task_id().expect("run owner");
        self.scheduler.get_task_mut(owner).root_shutdown_error = Some(error);
        let result = self.root_shutdown_return().expect("root obligations");
        match result? {
            super::AwaitResult::Yield(_) => {
                self.save_task_context(owner);
                self.current_frame.park(self.module_code);
                self.scheduler.set_current_task(None);
            }
            super::AwaitResult::FramePushed | super::AwaitResult::ValueReady(_) => {}
        }
        Ok(())
    }

    pub(super) fn finish_root_shutdown(&mut self, owner: TaskId) {
        let Some(task) = self.scheduler.task(owner) else {
            return;
        };
        let Some(id) = task.root_shutdown else {
            return;
        };
        let pending = if task.run_frame.is_none() {
            !task.gather_children.is_empty() || self.scheduler.gather_externals.values().any(|t| *t == owner)
        } else {
            !self.scheduler.run_tasks(owner).is_empty() || !self.scheduler.run_externals(owner).is_empty()
        };
        if pending {
            return;
        }
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        if !matches!(future.get(self.heap).state, ExternalFutureState::Pending { .. }) {
            return;
        }
        let ExternalFutureState::Pending { awaiter } = mem::replace(
            &mut future.get_mut(self.heap).state,
            ExternalFutureState::Resolved(Value::None),
        ) else {
            unreachable!()
        };
        drop(future);
        if let Some(awaiter) = awaiter {
            if let Some(error) = self.scheduler.get_task_mut(owner).root_shutdown_error.take() {
                self.scheduler.get_task_mut(owner).root_shutdown.take();
                Value::Ref(id).drop_with(self);
                self.deliver_awaiter_failure(awaiter, error);
            } else {
                self.deliver_awaiter_success(awaiter, Value::None);
            }
        }
    }

    pub(crate) fn new_task_group(&mut self) -> Value {
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.task_group = Some(TaskGroup {
            owner: None,
            state: TaskGroupState::Created,
            children: Vec::new(),
            body_error: None,
            interrupt: None,
            failure_pending: false,
            externals: Vec::new(),
            errors: Vec::new(),
        });
        Value::Ref(self.heap.allocate(HeapData::ExternalFuture(Box::new(future))))
    }

    pub(crate) fn task_group_method(&mut self, id: HeapId, attr: &EitherStr) -> Option<CallResult> {
        if attr.as_str(self.interns) != "create_task" {
            return None;
        }
        self.heap.inc_ref(id);
        Some(CallResult::Value(Value::Ref(self.heap.allocate(
            HeapData::BoundMethod(BoundMethod {
                instance: Value::Ref(id),
                func: Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::GroupCreateTask)),
            }),
        ))))
    }

    pub(crate) fn call_task_group(&mut self, id: HeapId, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        if attr.as_str(self.interns) != "create_task" {
            args.drop_with(self);
            return Err(ExcType::attribute_error("TaskGroup", attr.as_str(self.interns)));
        }
        let coro = args.get_one_arg("create_task", self.heap)?;
        let result = self.group_create_task(id, &coro).map(CallResult::Value);
        coro.drop_with(self);
        result
    }

    pub(crate) fn group_create_task(&mut self, id: HeapId, coro: &Value) -> RunResult<Value> {
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            unreachable!()
        };
        let group = future.task_group.as_ref().expect("group");
        let owner = group.owner;
        if group.state != TaskGroupState::Active || group.failure_pending || owner != self.scheduler.current_task_id() {
            return Err(
                SimpleException::new_msg(ExcType::RuntimeError, "TaskGroup admission is closed or foreign").into(),
            );
        }
        let value = self.spawn_public_task(coro)?;
        let Value::Ref(child) = value else { unreachable!() };
        let HeapData::ExternalFuture(future) = self.heap.get(child) else {
            unreachable!()
        };
        let task = future.public_task.as_ref().expect("Task").task.expect("live Task");
        let previous = self.scheduler.get_task_mut(task).lifetime_parent;
        if let Some(previous) = previous {
            self.scheduler
                .get_task_mut(previous)
                .gather_children
                .retain(|id| *id != task);
        }
        self.scheduler.get_task_mut(task).group_parent = Some(id);
        self.scheduler.get_task_mut(task).lifetime_parent = owner;
        self.scheduler
            .get_task_mut(owner.expect("owner"))
            .gather_children
            .push(task);
        self.heap.inc_ref(child);
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        future
            .get_mut(self.heap)
            .task_group
            .as_mut()
            .expect("group")
            .children
            .push(child);
        Ok(value)
    }

    pub(super) fn task_group_context(&mut self, id: HeapId, mode: u8, exc: Option<HeapId>) -> RunResult<CallResult> {
        let owner = self.scheduler.current_task_id().expect("current task");
        let internal = exc.is_some_and(|exc| {
            self.scheduler.get_task_mut(owner).groups.clone().iter().any(|id| {
                matches!(self.heap.get(*id), HeapData::ExternalFuture(f) if f.task_group.as_ref().is_some_and(|group|
                matches!(&group.interrupt, Some(Value::Ref(signal)) if *signal == exc)))
            })
        });
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let group = future.get_mut(self.heap).task_group.as_mut().expect("group");
        if mode == 0 {
            if group.state != TaskGroupState::Created {
                return Err(
                    SimpleException::new_msg(ExcType::RuntimeError, "TaskGroup has already been entered").into(),
                );
            }
            group.state = TaskGroupState::Active;
            group.owner = Some(owner);
            self.heap.inc_ref(id);
            self.scheduler.get_task_mut(owner).groups.push(id);
            self.heap.inc_ref(id);
            drop(future);
            return Ok(CallResult::Value(self.settled_awaitable(Value::Ref(id))));
        }
        if group.owner != Some(owner) || group.state != TaskGroupState::Active {
            return Err(SimpleException::new_msg(ExcType::RuntimeError, "TaskGroup exit is closed or foreign").into());
        }
        group.state = TaskGroupState::Exiting;
        // Our own interruption wakes the body but is not another user failure.
        let cancelled = matches!((&self.scheduler.get_task_mut(owner).child_cancel_signal, exc),
            (Some(Value::Ref(a)), Some(b)) if *a == b);
        if let Some(exc) = exc.filter(|_| !internal && !cancelled) {
            group.body_error = Some(Value::Ref(exc));
        }
        let cancel = exc.is_some() || group.failure_pending;
        let children = group.children.clone();
        if let Some(exc) = exc.filter(|_| !internal && !cancelled) {
            self.heap.inc_ref(exc);
        }
        drop(future);
        if cancel {
            self.cancel_group_children(&children);
        }
        self.finish_task_groups(owner);
        self.heap.inc_ref(id);
        Ok(CallResult::Value(Value::Ref(id)))
    }

    fn cancel_group_children(&mut self, children: &[HeapId]) {
        let tasks: Vec<_> = children
            .iter()
            .filter_map(|id| {
                let HeapData::ExternalFuture(f) = self.heap.get(*id) else {
                    unreachable!()
                };
                f.public_task.as_ref().and_then(|p| p.task)
            })
            .collect();
        for task in tasks {
            self.cancel_public_task(task);
        }
    }

    pub(super) fn task_group_child_completed(&mut self, id: HeapId, failed: bool) {
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let group = future.get_mut(self.heap).task_group.as_mut().expect("group");
        let owner = group.owner.expect("entered");
        let first = failed && !group.failure_pending;
        if failed {
            group.failure_pending = true;
        }
        let exiting = group.state == TaskGroupState::Exiting;
        let children = group.children.clone();
        drop(future);
        if first {
            self.cancel_group_children(&children);
            if !exiting {
                self.interrupt_group_owner(id, owner);
            }
        }
        self.finish_task_groups(owner);
    }

    /// Borrow cancellation's proven waiter detachment, but do not install a host
    /// terminal latch or permanently close the parent's task admission.
    fn interrupt_group_owner(&mut self, id: HeapId, owner: TaskId) {
        // Protecting the owner's exit wait does not shield ordinary work in
        // groups entered inside this failing group. Only descendants participate;
        // a caught inner failure must leave unrelated outer children alone.
        let groups = self.scheduler.get_task_mut(owner).groups.clone();
        if let Some(index) = groups.iter().position(|group| *group == id) {
            for nested in &groups[index + 1..] {
                let HeapData::ExternalFuture(future) = self.heap.get(*nested) else {
                    unreachable!()
                };
                let children = future.task_group.as_ref().expect("group").children.clone();
                self.cancel_group_children(&children);
            }
        }
        let task = self.scheduler.get_task_mut(owner);
        if task.native_cancel.is_some()
            || task.native_protected
            || task.gather_protected
            || task.drain.is_some()
            || self.scheduler.blocked_on(owner).is_none()
        {
            return;
        }
        let mut waits = vec![self.scheduler.blocked_on(owner).expect("body wait")];
        let mut targets = Vec::new();
        let mut handles = Vec::new();
        // Cancel only operands of the interrupted body's current await graph.
        // Unjoined outer-group Tasks must survive a caught inner failure.
        while let Some(wait) = waits.pop() {
            match self.heap.get(wait) {
                HeapData::GatherFuture(gather) => waits.extend(gather.items.iter().copied()),
                HeapData::Coroutine(_) => {
                    if let Some(task) = self.scheduler.coroutine_task(wait)
                        && !targets.contains(&task)
                    {
                        targets.push(task);
                    }
                }
                HeapData::ExternalFuture(future) => {
                    if let Some(task) = future.public_task.as_ref().and_then(|p| p.task)
                        && !targets.contains(&task)
                    {
                        targets.push(task);
                        handles.push(wait);
                    }
                }
                _ => {}
            }
        }
        self.cancel_child_task(owner, "TaskGroup child failed");
        let task = self.scheduler.get_task_mut(owner);
        task.native_cancel.take();
        let signal = task.child_cancel_signal.take();
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        future.get_mut(self.heap).task_group.as_mut().expect("group").interrupt = signal;
        for handle in handles {
            self.scheduler
                .public_gather_waits
                .entry(owner)
                .or_default()
                .extend(match self.heap.get(handle) {
                    HeapData::ExternalFuture(f) => f.public_task.as_ref().and_then(|p| p.task),
                    _ => None,
                });
            if !future
                .get(self.heap)
                .task_group
                .as_ref()
                .expect("group")
                .children
                .contains(&handle)
            {
                self.heap.inc_ref(handle);
                future
                    .get_mut(self.heap)
                    .task_group
                    .as_mut()
                    .expect("group")
                    .children
                    .push(handle);
            }
        }
        drop(future);
        for target in &targets {
            if !self.is_public_task(*target) {
                self.scheduler.get_task_mut(*target).group_parent = Some(id);
            }
            self.cancel_public_task_reason(*target, "TaskGroup child failed");
        }
        if !targets.is_empty() {
            let task = self.scheduler.get_task_mut(owner);
            task.group_body_waits = Some(targets);
            task.pending_gather_error = GatherErrorDelivery::Draining;
            self.scheduler.remove_from_ready_queue(owner);
            self.finish_child_drain(owner);
        }
    }

    /// Protected cleanup runs to its continuation before the pending group
    /// interruption can enter ordinary body exception routing.
    pub(super) fn pending_group_interrupt(&mut self) -> Option<RunError> {
        if self.current_frame.code.native_cleanup_at(self.instruction_ip)
            || self
                .suspended_frames
                .iter()
                .any(|f| f.code.native_cleanup_at(f.ip.saturating_sub(1)))
        {
            return None;
        }
        let groups = self.scheduler.current_task()?.groups.clone();
        for id in groups.into_iter().rev() {
            let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
                unreachable!()
            };
            let group = future.get(self.heap).task_group.as_ref().expect("group");
            if group.failure_pending && group.state == TaskGroupState::Active && group.interrupt.is_none() {
                let mut error: RunError =
                    SimpleException::new_msg(ExcType::ValueError, "TaskGroup child failed").into();
                error.materialize_object(self.heap);
                if let RunError::Exc(error) = &error {
                    let signal = error.object.as_ref().map(|v| v.clone_with_heap(self.heap));
                    future.get_mut(self.heap).task_group.as_mut().expect("group").interrupt = signal;
                }
                return Some(error);
            }
        }
        None
    }

    /// Complete innermost exits deterministically, retaining every non-cancellation
    /// outcome. ExceptionGroup / except* is not yet in the VM's exception model.
    pub(super) fn finish_task_groups(&mut self, owner: TaskId) {
        let Some(task) = self.scheduler.task(owner) else {
            return;
        };
        let groups = task.groups.clone();
        for id in groups.into_iter().rev() {
            let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
                unreachable!()
            };
            let group = future.get(self.heap).task_group.as_ref().expect("group");
            if group.state != TaskGroupState::Exiting {
                continue;
            }
            let children = group.children.clone();
            if children.iter().any(|child| {
                matches!(self.heap.get(*child), HeapData::ExternalFuture(f)
                if f.public_task.as_ref().is_some_and(|p| p.task.is_some()))
            }) {
                continue;
            }
            if self.scheduler.group_has_tasks(id)
                || group
                    .externals
                    .iter()
                    .any(|call| self.scheduler.gather_externals.contains_key(call))
            {
                continue;
            }
            let group = future.get_mut(self.heap).task_group.as_mut().expect("group");
            group.state = TaskGroupState::Finished;
            let body = group.body_error.take();
            let interrupt = group.interrupt.take();
            let children = mem::take(&mut group.children);
            let mut errors = mem::take(&mut group.errors);
            interrupt.drop_with(self.heap);
            for child in children {
                let HeapData::ExternalFuture(f) = self.heap.get(child) else {
                    unreachable!()
                };
                if !f.public_task.as_ref().expect("Task").cancelled
                    && let ExternalFutureState::Failed(RunError::Exc(error)) = &f.state
                    && let Some(value) = &error.object
                {
                    errors.push(value.clone_with_heap(self.heap));
                }
                Value::Ref(child).drop_with(self);
            }
            // A real body cancellation is re-raised by async-with when no child
            // fails. With failures it is retained as well; cancellation typing
            // remains the VM's documented ValueError subset.
            errors.extend(body);
            let result = if errors.is_empty() {
                Ok(Value::Bool(false))
            } else {
                // The public exception boundary exports metadata, not guest
                // argument graphs. Preserve every failure there as well as in
                // the exact, inspectable guest tuple; never choose one error.
                let summaries: Vec<_> = errors
                    .iter()
                    .map(|value| {
                        let Value::Ref(id) = value else {
                            unreachable!("exception object")
                        };
                        let HeapData::Exception(error) = self.heap.get(*id) else {
                            unreachable!("exception object")
                        };
                        error.simple.to_string()
                    })
                    .collect();
                let exceptions = allocate_tuple(errors.into_iter().collect(), self.heap);
                let args = allocate_tuple(smallvec::smallvec![exceptions], self.heap);
                let simple = SimpleException::new_msg(
                    ExcType::RuntimeError,
                    format!(
                        "unhandled errors in a TaskGroup (ExceptionGroup unsupported): {}",
                        summaries.join("; ")
                    ),
                );
                let mut raised = ExceptionRaise::from(simple.clone());
                raised.object = Some(Value::Ref(self.heap.allocate(HeapData::Exception(Box::new(
                    ExceptionObject {
                        simple,
                        args: Some(args),
                    },
                )))));
                Err(RunError::Exc(raised))
            };
            let state = match &result {
                Ok(v) => ExternalFutureState::Resolved(v.clone_with_heap(self.heap)),
                Err(e) => ExternalFutureState::Failed(e.clone_with_heap(self.heap)),
            };
            let ExternalFutureState::Pending { awaiter } = mem::replace(&mut future.get_mut(self.heap).state, state)
            else {
                unreachable!()
            };
            drop(future);
            let roots = &mut self.scheduler.get_task_mut(owner).groups;
            roots.retain(|root| *root != id);
            Value::Ref(id).drop_with(self);
            match (awaiter, result) {
                (Some(waiter), Ok(v)) => {
                    self.deliver_awaiter_success(waiter, v);
                }
                (Some(waiter), Err(e)) => {
                    self.deliver_awaiter_failure(waiter, e);
                }
                (None, Ok(v)) => v.drop_with(self),
                (None, Err(e)) => e.drop_with(self),
            }
        }
    }
}
