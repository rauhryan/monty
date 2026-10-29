//! Experimental task-owned children. Scheduling stays in Scheduler; futures own outcomes.
use std::mem;

#[cfg(test)]
use super::scheduler::GatherErrorDelivery;
use super::{AwaitResult, VM, scheduler::NativeCancellation};
use crate::{
    asyncio::{Awaiter, CallId, ChildIdentity, CoroutineState, ExternalFuture, ExternalFutureState, TaskId},
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    types::List,
    value::Value,
};

impl VM<'_> {
    pub(crate) fn open_child_scope(&mut self) -> RunResult<()> {
        let owner = self.scheduler.current_task_id().expect("current task");
        let host_cancelled = self.native_cancel_reason().is_some();
        let task = self.scheduler.get_task_mut(owner);
        if host_cancelled || task.scope_open || task.drain.is_some() || task.native_cancel.is_some() {
            return Err(ExcType::value_error("nested or cancelled child scope is not supported"));
        }
        task.scope_token.take().drop_with(self.heap);
        task.scope_token = Some(Value::Ref(self.heap.allocate(HeapData::List(List::new(Vec::new())))));
        task.scope_open = true;
        Ok(())
    }

    pub(crate) fn spawn_owned_child(&mut self, value: &Value) -> RunResult<Value> {
        let owner = self.scheduler.current_task_id().expect("current task");
        let host_cancelled = self.native_cancel_reason().is_some();
        let task = self.scheduler.get_task_mut(owner);
        if host_cancelled || !task.scope_open || task.drain.is_some() || task.native_cancel.is_some() {
            return Err(ExcType::value_error("child admission is closed"));
        }
        let Some(Value::Ref(scope)) = task.scope_token.as_ref() else {
            unreachable!()
        };
        let scope = *scope;
        let Value::Ref(coroutine) = value else {
            return Err(ExcType::type_error("_spawn requires an ordinary coroutine binding"));
        };
        if !matches!(self.heap.get(*coroutine), HeapData::Coroutine(c) if c.state == CoroutineState::New) {
            return Err(ExcType::type_error("_spawn requires a new ordinary coroutine binding"));
        }
        let task = self
            .scheduler
            .spawn(self.heap, *coroutine, None)
            .ok_or_else(ExcType::cannot_reuse_already_awaited_coroutine)?;
        self.heap.inc_ref(scope);
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), Some(Value::Ref(scope)));
        future.child = Some(ChildIdentity {
            owner,
            scope,
            task: Some(task),
            consumed: false,
            cancelled: false,
        });
        let id = self.heap.allocate(HeapData::ExternalFuture(Box::new(future)));
        // Three owners: caller's Value, parent's scope root, live child's result root.
        self.heap.inc_ref(id);
        self.heap.inc_ref(id);
        self.scheduler.get_task_mut(owner).children.push(id);
        self.scheduler.get_task_mut(task).child_result = Some(id);
        self.scheduler.get_task_mut(task).lifetime_parent = Some(owner);
        Ok(Value::Ref(id))
    }

    fn owned_identity(&self, value: &Value) -> RunResult<(HeapId, Option<TaskId>)> {
        let Value::Ref(id) = value else {
            return Err(ExcType::value_error("unknown child handle"));
        };
        let HeapData::ExternalFuture(future) = self.heap.get(*id) else {
            return Err(ExcType::value_error("unknown child handle"));
        };
        let Some(child) = &future.child else {
            return Err(ExcType::value_error("unknown child handle"));
        };
        if Some(child.owner) != self.scheduler.current_task_id()
            || self
                .scheduler
                .current_task()
                .is_none_or(|task| !matches!(task.scope_token, Some(Value::Ref(id)) if id == child.scope))
        {
            return Err(ExcType::value_error("foreign child handle"));
        }
        Ok((*id, child.task))
    }

    /// Cancelling a completed owned handle is a no-op, without a completed-task history.
    pub(crate) fn cancel_owned_child(&mut self, value: &Value, reason: &str) -> RunResult<()> {
        let (_, task) = self.owned_identity(value)?;
        if let Some(task) = task {
            self.cancel_owned_subtree(task, reason);
        }
        Ok(())
    }

    /// Logical cancellation follows lifetime links even after gather first-error
    /// delivery. Detach parent routes before signalling descendants.
    fn cancel_owned_subtree(&mut self, root: TaskId, reason: &str) {
        for id in self.scheduler.owned_subtree(root, self.heap) {
            self.cancel_child_task(id, reason);
        }
    }

    /// Latch before delivery; a drain barrier always retains its legitimate reply.
    pub(super) fn cancel_child_task(&mut self, id: TaskId, reason: &str) {
        if self.scheduler.get_task_mut(id).native_cancel.is_none() {
            self.scheduler.get_task_mut(id).native_cancel = Some(NativeCancellation {
                reason: reason.to_owned(),
                delivered: false,
                revoked: None,
                policy: 7,
                interrupted_error: None,
            });
        }
        self.deliver_task_cancel(id);
        let task = self.scheduler.get_task_mut(id);
        if !task.native_protected && !task.gather_protected && task.drain.is_none() {
            let calls: Vec<_> = self
                .scheduler
                .gather_externals
                .iter()
                .filter_map(|(call, owner)| (*owner == id).then_some(*call))
                .collect();
            for call in calls {
                // Revocation is logical settlement, not task disposal. There is
                // no authored continuation in a direct external gather item.
                self.scheduler.gather_externals.remove(&call);
                self.fail_future(call.raw(), ExcType::value_error(reason));
            }
        }
    }

    /// Detach only this task's waiter. A cancelled join must not consume a child error
    /// or leave a waiter that can later wake the owner's unrelated cleanup operation.
    pub(super) fn deliver_task_cancel(&mut self, id: TaskId) {
        let task = self.scheduler.get_task_mut(id);
        if task.native_protected
            || task.gather_protected
            || task.drain.is_some()
            || task.native_cancel.as_ref().is_none_or(|c| c.delivered)
        {
            return;
        }
        let Some(wait) = self.scheduler.blocked_on(id) else {
            return;
        };
        let (external, call_id) = if let HeapReadOutput::ExternalFuture(future) = self.heap.read(wait) {
            let external = future.get(self.heap).child.is_none();
            let call_id = future.get(self.heap).call_id.raw();
            if !external && let ExternalFutureState::Pending { awaiter } = &mut future.get_mut(self.heap).state {
                awaiter.take().drop_with(self.heap);
            }
            (external, call_id)
        } else {
            // Failing the route retains first-error semantics and releases its
            // partial results. Lifetime links live in Scheduler, not this state.
            let HeapReadOutput::GatherFuture(gather) = self.heap.read(wait) else {
                unreachable!()
            };
            let reason = &self
                .scheduler
                .get_task_mut(id)
                .native_cancel
                .as_ref()
                .expect("cancel latch")
                .reason;
            let error = ExcType::value_error(reason);
            let awaiter = gather.fail(self.heap, &error);
            drop(gather);
            error.drop_with(self);
            awaiter.drop_with(self);
            (false, 0)
        };
        if !external {
            self.scheduler
                .get_task_mut(id)
                .pending_join
                .take()
                .map(Value::Ref)
                .drop_with(self.heap);
        }
        let diagnostic = self.task_cancel_diagnostic(id);
        let cancel = self
            .scheduler
            .get_task_mut(id)
            .native_cancel
            .as_mut()
            .expect("cancel latch");
        cancel.delivered = true;
        cancel.interrupted_error = diagnostic;
        if external {
            cancel.revoked = Some(call_id);
        }
        let mut signal = ExcType::value_error(&cancel.reason);
        signal.materialize_object(self.heap);
        if let RunError::Exc(error) = &signal {
            self.scheduler.get_task_mut(id).child_cancel_signal =
                error.object.as_ref().map(|value| value.clone_with_heap(self.heap));
        }
        if external {
            self.fail_future(call_id, signal);
        } else {
            self.deliver_awaiter_failure(Awaiter::Task(id), signal);
        }
    }

    /// Register a join; consumption happens only when execution receives its outcome.
    pub(super) fn prepare_child_join(&mut self, id: HeapId, awaiter: &Awaiter) -> RunResult<()> {
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            unreachable!()
        };
        let Some(child) = &future.child else { return Ok(()) };
        let owner = child.owner;
        let scope = child.scope;
        if self
            .scheduler
            .current_task()
            .is_none_or(|task| !matches!(task.scope_token, Some(Value::Ref(id)) if id == scope))
            || !matches!(awaiter, Awaiter::Task(task) if *task == owner)
            || self.scheduler.current_task_id() != Some(owner)
        {
            return Err(ExcType::value_error("foreign child handle"));
        }
        let pending = matches!(future.state, ExternalFutureState::Pending { .. });
        if pending {
            if self.scheduler.get_task_mut(owner).pending_join.is_some() {
                return Err(ExcType::value_error("join already pending"));
            }
            self.heap.inc_ref(id);
            self.scheduler.get_task_mut(owner).pending_join = Some(id);
        } else {
            self.consume_child(id, owner);
        }
        Ok(())
    }

    pub(super) fn consume_child_delivery(&mut self, task: TaskId) {
        if let Some(id) = self.scheduler.get_task_mut(task).pending_join.take() {
            self.consume_child(id, task);
            Value::Ref(id).drop_with(self);
        }
    }

    fn consume_child(&mut self, id: HeapId, owner: TaskId) {
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        future.get_mut(self.heap).child.as_mut().expect("child").consumed = true;
        drop(future);
        self.release_child_scope_root(id, owner);
    }

    fn release_child_scope_root(&mut self, id: HeapId, owner: TaskId) {
        let children = &mut self.scheduler.get_task_mut(owner).children;
        if let Some(index) = children.iter().position(|child| *child == id) {
            children.remove(index);
            Value::Ref(id).drop_with(self);
        }
    }

    /// Close admission, cancel children in spawn order, then await all cleanup.
    /// The result contains unconsumed ordinary exception objects, in spawn order.
    pub(crate) fn drain_owned_children(&mut self) -> RunResult<Value> {
        let owner = self.scheduler.current_task_id().expect("current task");
        if !self.scheduler.get_task_mut(owner).scope_open || self.scheduler.get_task_mut(owner).drain.is_some() {
            return Err(ExcType::value_error("scope drain already pending"));
        }
        let future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        let id = self.heap.allocate(HeapData::ExternalFuture(Box::new(future)));
        self.scheduler.get_task_mut(owner).drain = Some(id);
        self.scheduler.get_task_mut(owner).scope_open = false;
        let children = self.scheduler.get_task_mut(owner).children.clone();
        for child in children {
            let HeapData::ExternalFuture(future) = self.heap.get(child) else {
                unreachable!()
            };
            if let Some(task) = future.child.as_ref().expect("child").task {
                self.cancel_owned_subtree(task, "scope-exit");
            }
        }
        let gathered = self.scheduler.get_task_mut(owner).gather_children.clone();
        for task in gathered {
            self.cancel_owned_subtree(task, "scope-exit");
        }
        let calls: Vec<_> = self
            .scheduler
            .gather_externals
            .iter()
            .filter_map(|(call, task)| (*task == owner).then_some(*call))
            .collect();
        for call in calls {
            self.scheduler.gather_externals.remove(&call);
            self.fail_future(call.raw(), ExcType::value_error("scope-exit"));
        }
        self.heap.inc_ref(id);
        self.finish_child_drain(owner);
        Ok(Value::Ref(id))
    }

    pub(super) fn complete_owned_child(&mut self, mut result: RunResult<Value>) -> RunResult<AwaitResult> {
        let task = self.scheduler.current_task_id().expect("child task");
        let record = self.scheduler.get_task_mut(task);
        let id = record.child_result.take().expect("owned child result");
        // Terminal cancelled even if guest caught the one delivery and returned.
        let cancelled = record.native_cancel.is_some()
            && match &result {
                Ok(_) => true,
                Err(RunError::Exc(error)) => {
                    matches!((&record.child_cancel_signal, &error.object), (Some(Value::Ref(a)), Some(Value::Ref(b))) if a == b)
                }
                Err(_) => false,
            };
        if let Some(cancel) = &record.native_cancel
            && let Ok(value) = result
        {
            value.drop_with(self.heap);
            result = Err(ExcType::value_error(&cancel.reason));
        }
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let child = future.get_mut(self.heap).child.as_mut().expect("child");
        child.task = None;
        child.cancelled = cancelled;
        let owner = child.owner;
        // Older direct-owned snapshots predate the general lifetime backlink.
        self.scheduler.get_task_mut(task).lifetime_parent = Some(owner);
        if let Err(error) = &mut result {
            error.materialize_object(self.heap);
        }
        let state = match &result {
            Ok(value) => ExternalFutureState::Resolved(value.clone_with_heap(self.heap)),
            Err(error) => ExternalFutureState::Failed(error.clone_with_heap(self.heap)),
        };
        let ExternalFutureState::Pending { awaiter } = mem::replace(&mut future.get_mut(self.heap).state, state) else {
            unreachable!()
        };
        drop(future);
        // Successful/cancelled completions carry no unconsumed error obligation.
        // Their handles retain outcomes; the scope needs no completed history.
        if result.is_ok() || cancelled {
            self.release_child_scope_root(id, owner);
        }
        Value::Ref(id).drop_with(self);
        self.scheduler.transfer_gather_lifetime(task);
        self.cleanup_current_task();
        self.scheduler.cancel_task(task, self.heap);
        let delivery = match (awaiter, result) {
            (Some(awaiter), Ok(value)) => self.deliver_awaiter_success(awaiter, value),
            (Some(awaiter), Err(error)) => self.deliver_awaiter_failure(awaiter, error),
            (None, Ok(value)) => {
                value.drop_with(self);
                None
            }
            (None, Err(error)) => {
                error.drop_with(self);
                None
            }
        };
        self.finish_child_drain(owner);
        if self.resume_after_task_exit(delivery)? {
            Ok(AwaitResult::FramePushed)
        } else {
            Ok(AwaitResult::Yield(self.scheduler.pending_call_ids()))
        }
    }

    pub(super) fn finish_child_drain(&mut self, owner: TaskId) {
        let external = self.scheduler.gather_externals.values().any(|task| *task == owner);
        let task = self.scheduler.get_task_mut(owner);
        if task.drain.is_none() || !task.gather_children.is_empty() || external {
            return;
        }
        for id in &task.children {
            let HeapData::ExternalFuture(future) = self.heap.get(*id) else {
                unreachable!()
            };
            if future.child.as_ref().expect("child").task.is_some() {
                return;
            }
        }
        let id = task.drain.take().expect("drain");
        let children = mem::take(&mut task.children);
        let mut errors = mem::take(&mut task.gather_errors);
        for child in children {
            let HeapData::ExternalFuture(future) = self.heap.get(child) else {
                unreachable!()
            };
            if let ExternalFutureState::Failed(RunError::Exc(error)) = &future.state {
                // Scope-exit cancellation is expected; ordinary failures remain visible.
                if !future.child.as_ref().expect("child").cancelled
                    && let Some(object) = &error.object
                {
                    errors.push(object.clone_with_heap(self.heap));
                }
            }
            Value::Ref(child).drop_with(self);
        }
        let value = Value::Ref(self.heap.allocate(HeapData::List(List::new(errors))));
        let HeapReadOutput::ExternalFuture(future) = self.heap.read(id) else {
            unreachable!()
        };
        let cached = value.clone_with_heap(self.heap);
        let ExternalFutureState::Pending { awaiter } = mem::replace(
            &mut future.get_mut(self.heap).state,
            ExternalFutureState::Resolved(cached),
        ) else {
            unreachable!()
        };
        drop(future);
        Value::Ref(id).drop_with(self);
        if let Some(awaiter) = awaiter {
            self.deliver_awaiter_success(awaiter, value);
        } else {
            value.drop_with(self);
        }
    }
}

#[cfg(test)]
impl VM<'_> {
    /// Exercise cancellation after a gather wakes its owner but before delivery.
    pub(crate) fn ownership_probe_gather_delivery(&mut self, cancel: bool) {
        let (task, gather) = self
            .scheduler
            .owned_task_tree(self.heap)
            .into_iter()
            .find_map(|task| {
                let wait = self.scheduler.blocked_on(task)?;
                matches!(self.heap.get(wait), HeapData::GatherFuture(_)).then_some((task, wait))
            })
            .expect("parked gather");
        self.heap.inc_ref(gather);
        let route = Awaiter::GatherSlot { gather, source: gather };
        assert_eq!(
            self.deliver_awaiter_failure(route, ExcType::value_error("undelivered-gather")),
            Some(task)
        );
        assert_eq!(
            self.scheduler.get_task_mut(task).pending_gather_error,
            GatherErrorDelivery::Pending
        );
        assert!(self.scheduler.get_task_mut(task).gather_errors.is_empty());
        if cancel {
            self.cancel_child_task(task, "guest");
        }
        self.scheduler.remove_from_ready_queue(task);
        let error = self.activate_task(task).expect_err("queued error or cancellation");
        error.drop_with(self);
        assert_eq!(
            self.scheduler.get_task_mut(task).pending_gather_error,
            GatherErrorDelivery::None
        );
        assert_eq!(
            self.scheduler.get_task_mut(task).gather_errors.len(),
            usize::from(cancel)
        );
        eprintln!(
            "gather wake consumed=false; cancelled-before-delivery={cancel}; retained={}",
            usize::from(cancel)
        );
    }

    /// Probe the wake/delivery gap with a real suspended join, then abandon it.
    pub(crate) fn ownership_probe_join_delivery(&mut self) {
        let owner = TaskId::default();
        let id = self.scheduler.get_task_mut(owner).pending_join.expect("pending join");
        let delivered = self.deliver_awaiter_failure(Awaiter::Task(owner), ExcType::value_error("probe"));
        assert_eq!(delivered, Some(owner));
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            unreachable!()
        };
        assert!(!future.child.as_ref().unwrap().consumed, "wake is not delivery");
        self.scheduler.remove_from_ready_queue(owner);
        let error = self.activate_task(owner).expect_err("join must deliver the error");
        error.drop_with(self);
        let HeapData::ExternalFuture(future) = self.heap.get(id) else {
            unreachable!()
        };
        assert!(future.child.as_ref().unwrap().consumed, "selected join receives error");
        assert!(!self.scheduler.get_task_mut(owner).children.contains(&id));
        eprintln!("owned-child wake consumed=false; delivered consumed=true; scope root removed");
    }
}
