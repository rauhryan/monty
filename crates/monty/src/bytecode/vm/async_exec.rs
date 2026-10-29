//! Async execution support for the VM.
//!
//! This module contains all async-related methods for the VM including:
//! - Awaiting coroutines, external futures, and gather futures
//! - Task scheduling and context switching
//! - Task completion and failure handling
//! - External future resolution

use std::{mem, task::Poll};

use monty_types::{InvalidInputError, MontyException, ResourceError, ResourceTracker};
use smallvec::{SmallVec, smallvec};

#[cfg(test)]
use super::exceptions::CaughtException;
use super::{
    AwaitResult, CallFrame, FrameExit, Opcode, VM, frame_code, function_namespace, recursion::frame_recursion_depth,
    stack_index,
};
use crate::{
    asyncio::{
        AwaitedGather, Awaiter, CallId, Coroutine, CoroutineState, ExternalFuture, ExternalFutureState, GatherFuture,
        GatherState, PendingChildren, TaskGroupState, TaskId,
    },
    bytecode::{
        Code,
        vm::scheduler::{GatherErrorDelivery, SerializedTaskFrame},
    },
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    heap::{
        ContainsHeap, DropGuard, DropWithContext, HeapData, HeapId, HeapObjectRead, HeapReadMut, HeapReadOutput,
        HeapReader,
    },
    intern::FunctionId,
    object_bridge::MontyObjectExt,
    run_progress::{ExtFunctionResult, ExtFunctionResultExt},
    types::List,
    value::Value,
};

impl<'h> VM<'h> {
    /// Allows eager host resolution only for an immediate await with no competing work.
    pub(crate) fn allow_eager_await(&self) -> bool {
        let next = self.current_frame.bytecode.get(self.current_frame.ip);
        next == Some(&(Opcode::Await as u8)) && self.scheduler.can_await_eagerly()
    }

    /// Executes the Await opcode.
    ///
    /// Pops the awaitable from the stack and handles it based on its type:
    /// - `Coroutine`: validates state is New, then pushes a frame to execute it
    /// - `ExternalFuture`: blocks until resolved or yields if not ready
    /// - `GatherFuture`: spawns tasks for coroutines and tracks external futures
    ///
    /// Returns `AwaitResult` indicating what action the VM should take.
    pub(super) fn exec_get_awaitable(&mut self) -> Result<AwaitResult, RunError> {
        let this = self;
        let awaitable = this.pop();
        defer_drop!(awaitable, this);

        let awaiter = Awaiter::Task(
            this.scheduler
                .current_task_id()
                .expect("exec_get_awaitable called without a current task"),
        );

        match awaitable {
            Value::Ref(heap_id) => {
                let heap_id = *heap_id;
                let poll = match this.heap.read(heap_id) {
                    HeapReadOutput::Coroutine(coro) => return this.await_coroutine(&coro),
                    HeapReadOutput::GatherFuture(gather) => match this.await_gather_future(gather, awaiter) {
                        Ok(poll) => poll,
                        Err(error) => {
                            // Commit can fail after admitting earlier operands
                            // (for example a reused coroutine in a later slot).
                            // Such children obey the same cleanup barrier as
                            // children of an asynchronously failed gather.
                            if let Some((task_id, true)) = this.gather_lifetime_context() {
                                this.scheduler.get_task_mut(task_id).native_protected = true;
                                this.scheduler.block_current_on(heap_id, this.heap);
                                this.scheduler.make_ready(task_id, Err(error), this.heap);
                                if this.hold_protected_gather_error(task_id) {
                                    return this.switch_or_yield();
                                }
                                this.scheduler.remove_from_ready_queue(task_id);
                                return Err(this.scheduler.take_resume_result(task_id).unwrap_err());
                            }
                            return Err(error);
                        }
                    },
                    HeapReadOutput::ExternalFuture(fut) => this.await_external_future(&fut, awaiter)?,
                    _ => return Err(ExcType::object_not_awaitable(&awaitable.py_type_name(this))),
                };
                match poll {
                    Poll::Ready(value) => Ok(AwaitResult::ValueReady(value)),
                    Poll::Pending => {
                        let protected = this
                            .suspended_frames
                            .iter()
                            .chain([&this.current_frame])
                            .any(|f| f.code.native_cleanup_at(f.ip.saturating_sub(1)));
                        let task_id = this.scheduler.current_task_id().expect("awaiting task");
                        this.scheduler.get_task_mut(task_id).native_protected = protected;
                        this.scheduler.block_current_on(heap_id, this.heap);
                        this.wake_queue_operation(heap_id);
                        this.switch_or_yield()
                    }
                }
            }
            _ => Err(ExcType::object_not_awaitable(&awaitable.py_type_name(this))),
        }
    }

    /// Awaits a coroutine by pushing a frame to execute it.
    ///
    /// Validates the coroutine is in `New` state, extracts its captured namespace
    /// and cells, marks it as `Running`, and pushes a frame to execute the coroutine body.
    fn await_coroutine(&mut self, coro: &HeapObjectRead<'h, Coroutine>) -> Result<AwaitResult, RunError> {
        // Check if coroutine can be awaited (must be New)
        if coro.get(self.heap).state != CoroutineState::New || self.scheduler.has_coroutine_task(coro.id()) {
            return Err(ExcType::cannot_reuse_already_awaited_coroutine());
        }

        // Extract coroutine data before mutating
        let func_id = coro.get(self.heap).func_id;
        self.admit_native_code(&self.interns.get_function(func_id).code)?;
        let globals = coro.get(self.heap).globals;
        let namespace_values: Vec<Value> = coro
            .get(self.heap)
            .namespace
            .iter()
            .map(|v| v.clone_with_heap(self))
            .collect();

        // Mark coroutine as Running
        coro.get_mut(self.heap).state = CoroutineState::Running;

        // Create namespace and push frame (guard drops awaitable at scope exit)
        self.start_coroutine_frame(func_id, namespace_values, globals)?;

        Ok(AwaitResult::FramePushed)
    }

    /// Awaits a gather future from the user's `await gather` site.
    ///
    /// A settled gather replays its cached result; a `Pending` one is handed to
    /// [`Self::commit_gather_tree`], which commits it along with every gather
    /// nested inside it.
    fn await_gather_future(
        &mut self,
        gather: HeapObjectRead<'h, GatherFuture>,
        awaiter: Awaiter,
    ) -> Result<Poll<Value>, RunError> {
        let mut awaiter_guard = DropGuard::new(awaiter, self);
        let this = awaiter_guard.ctx();
        if let Some(value) = poll_settled_gather(&gather, this.heap)? {
            Ok(Poll::Ready(value))
        } else {
            if let Some((owner, false)) = this.gather_lifetime_context()
                && (this.native_cancel_reason().is_some()
                    || this
                        .scheduler
                        .task(owner)
                        .is_some_and(|task| task.native_cancel.is_some()))
            {
                return Err(ExcType::value_error("child admission is closed"));
            }
            // The walk re-reads each gather by id, so hand the handle back
            // before it starts.
            let gather_id = gather.id();
            drop(gather);
            let (awaiter, this) = awaiter_guard.into_parts();
            this.commit_gather_tree(gather_id, awaiter)
        }
    }

    /// Commits `root` and every gather nested inside it, reporting whether the
    /// root settled synchronously.
    ///
    /// Nesting a gather costs no Python frames (`g = asyncio.gather(g)` in a
    /// loop), so the recursion limit never saw the commit walk that descends
    /// through it — a recursive walk turned heap-held nesting back into native
    /// frames and aborted the process. The frames it needed now live in
    /// `stack`, one [`GatherCommit`] per level, which makes nesting depth a
    /// matter of memory rather than of native stack, as it is for CPython's
    /// loop-driven futures.
    fn commit_gather_tree(&mut self, root: HeapId, awaiter: Awaiter) -> Result<Poll<Value>, RunError> {
        let mut stack = vec![self.open_gather_commit(root, awaiter, None)];
        let mut steps = 0;
        let outcome = loop {
            // Each level costs a frame on the way down and a result list on the
            // way back up, and the whole walk runs no bytecode, so
            // `dispatch_checkpoint` never sees how far it has grown. Without
            // this check a nest that fits under `max_memory` commits its way
            // past the allocator's hard ceiling, taking the worker down instead
            // of ending the run.
            if let Err(err) = self.heap.tracker.check_memory_time_every(steps) {
                break Err(err.into());
            }
            steps += 1;

            let frame = stack.last_mut().expect("the commit stack is emptied only by returning");
            match self.step_gather_commit(frame) {
                // Descend: the nested gather's own items must be committed
                // before the frame that owns its result slot can continue.
                Ok(Some(nested)) => {
                    if let Err(err) = check_commit_stack_growth(stack.len(), stack.capacity(), &self.heap.tracker) {
                        nested.drop_with(self.heap);
                        break Err(err.into());
                    }
                    stack.push(nested);
                }
                Ok(None) => {
                    let frame = stack.pop().expect("the frame just stepped is still on the stack");
                    let (child_id, slot) = (frame.gather, frame.parent_slot);
                    let poll = self.settle_gather_commit(frame);
                    match stack.last_mut() {
                        // The root settled: `poll` is what the `await` site sees.
                        None => break Ok(poll),
                        Some(parent) => {
                            let slot = slot.expect("only the root commit frame has no parent slot");
                            parent.resume_after_child(child_id, slot, poll);
                        }
                    }
                }
                Err(err) => break Err(err),
            }
        };

        match outcome {
            Ok(poll) => Ok(poll),
            Err(mut err) => {
                err.materialize_object(self.heap);
                self.unwind_gather_commits(stack, &err);
                Err(err)
            }
        }
    }

    /// Opens a commit frame for the `Pending` gather `gather`, sized to its items.
    fn open_gather_commit(&mut self, gather: HeapId, awaiter: Awaiter, parent_slot: Option<usize>) -> GatherCommit {
        let HeapReadOutput::GatherFuture(item_source) = self.heap.read(gather) else {
            panic!("gather commit frame id is not a GatherFuture")
        };
        let item_count = item_source.get(self.heap).item_count();
        GatherCommit {
            gather,
            awaiter,
            parent_slot,
            next: 0,
            results: (0..item_count).map(|_| None).collect(),
            pending_children: PendingChildren::new(),
        }
    }

    /// Commits `frame`'s items left-to-right into result slots or pending
    /// children, spawning coroutine children and installing awaiters on
    /// external futures as it goes.
    ///
    /// Stops early at a `Pending` nested gather, returning the frame the caller
    /// must commit before this one can continue; `Ok(None)` means every item is
    /// committed. Any error leaves the items handled so far in `frame`, which
    /// [`Self::unwind_gather_commits`] settles.
    fn step_gather_commit(&mut self, frame: &mut GatherCommit) -> Result<Option<GatherCommit>, RunError> {
        let lifetime = self.gather_lifetime_context();
        let HeapReadOutput::GatherFuture(gather) = self.heap.read(frame.gather) else {
            panic!("gather commit frame id is not a GatherFuture")
        };
        let gather_id = frame.gather;

        while frame.next < frame.results.len() {
            let idx = frame.next;
            let item_id = gather.get(self.heap).items[idx];
            if let Some(slots) = frame.pending_children.get_mut(&item_id) {
                // Dedup: We've already registered this item in this commit pass —
                // this is a duplicate item (e.g. `gather(coro, coro)`). Just
                // append the new slot index to the existing entry.
                slots.push(idx);
                frame.next += 1;
                continue;
            }

            let poll = match self.heap.read(item_id) {
                HeapReadOutput::Coroutine(coro) => {
                    // Reject reuse up-front: either the coroutine is no longer
                    // `New`, or another gather already spawned it (`spawn`
                    // returns `Ok(None)`).
                    if coro.get(self.heap).state != CoroutineState::New {
                        return Err(ExcType::cannot_reuse_already_awaited_coroutine());
                    }
                    let Some(child) = self.scheduler.spawn(self.heap, item_id, Some(gather_id)) else {
                        return Err(ExcType::cannot_reuse_already_awaited_coroutine());
                    };
                    if let Some((owner, protected)) = lifetime {
                        self.scheduler.get_task_mut(owner).gather_children.push(child);
                        let group = self.scheduler.get_task_mut(owner).group_parent;
                        let task = self.scheduler.get_task_mut(child);
                        task.group_parent = group;
                        task.lifetime_parent = Some(owner);
                        task.gather_protected = protected;
                        task.cleanup_parent_frame = (!protected).then_some(self.suspended_frames.len());
                    }
                    Poll::Pending
                }
                HeapReadOutput::ExternalFuture(fut) => {
                    self.heap.inc_ref(gather_id);
                    let sub_awaiter = Awaiter::GatherSlot {
                        gather: gather_id,
                        source: item_id,
                    };
                    let poll = self.await_external_future(&fut, sub_awaiter)?;
                    if poll.is_pending()
                        && let Some((owner, true)) = lifetime
                        && let Some(task) = fut.get(self.heap).public_task.as_ref().and_then(|task| task.task)
                    {
                        let waits = self.scheduler.public_gather_waits.entry(owner).or_default();
                        if !waits.contains(&task) {
                            waits.push(task);
                        }
                    }
                    if poll.is_pending()
                        && fut.get(self.heap).public_task.is_none()
                        && let Some((owner, _)) = lifetime
                    {
                        let call = fut.get(self.heap).call_id;
                        self.scheduler.gather_externals.insert(call, owner);
                        if lifetime.is_some_and(|(_, protected)| !protected) {
                            self.scheduler
                                .get_task_mut(owner)
                                .cleanup_externals
                                .push((call, self.suspended_frames.len()));
                        }
                        if let Some(group) = self.scheduler.get_task_mut(owner).group_parent {
                            let HeapReadOutput::ExternalFuture(group) = self.heap.read(group) else {
                                unreachable!()
                            };
                            group
                                .get_mut(self.heap)
                                .task_group
                                .as_mut()
                                .expect("group")
                                .externals
                                .push(call);
                        }
                    }
                    poll
                }
                HeapReadOutput::GatherFuture(child_gather) => {
                    if let Some(value) = poll_settled_gather(&child_gather, self.heap)? {
                        Poll::Ready(value)
                    } else {
                        drop(child_gather);
                        // Both the inc_ref and the awaiter that owns it are
                        // handed to the nested frame, which releases them when
                        // it settles; until then nothing is committed for this
                        // slot, so an error above needs no cleanup here.
                        self.heap.inc_ref(gather_id);
                        let sub_awaiter = Awaiter::GatherSlot {
                            gather: gather_id,
                            source: item_id,
                        };
                        return Ok(Some(self.open_gather_commit(item_id, sub_awaiter, Some(idx))));
                    }
                }
                _ => panic!("gather item is not a Coroutine, ExternalFuture, or GatherFuture"),
            };

            match poll {
                Poll::Ready(value) => frame.results[idx] = Some(value),
                Poll::Pending => {
                    frame.pending_children.insert(item_id, smallvec![idx]);
                }
            }
            frame.next += 1;
        }

        Ok(None)
    }

    /// Every ordinary gather installs durable lifetime links, even if its first
    /// error is caught and its creator completes before the remaining operands.
    fn gather_lifetime_context(&self) -> Option<(TaskId, bool)> {
        let task = self.scheduler.current_task()?;
        let protected = task.gather_protected
            || self
                .suspended_frames
                .iter()
                .chain([&self.current_frame])
                .any(|f| f.code.native_cleanup_at(f.ip.saturating_sub(1)));
        Some((task.id, protected))
    }

    /// Settles a fully-committed frame.
    ///
    /// With nothing left in flight the gather goes straight to `Completed` with
    /// its result list (this covers the empty `gather()` too); otherwise it
    /// parks in `Awaited`, taking over the frame's bookkeeping, and later
    /// resolutions drive it from there.
    fn settle_gather_commit(&mut self, frame: GatherCommit) -> Poll<Value> {
        let HeapReadOutput::GatherFuture(gather) = self.heap.read(frame.gather) else {
            panic!("gather commit frame id is not a GatherFuture")
        };

        if frame.pending_children.is_empty() {
            let results: Vec<Value> = frame
                .results
                .into_iter()
                .map(|r| r.expect("all results filled for synchronous gather completion"))
                .collect();
            let list_id = self.heap.allocate(HeapData::List(List::new(results)));
            gather.cache_result(self.heap, list_id);
            frame.awaiter.drop_with(self.heap);
            Poll::Ready(Value::Ref(list_id))
        } else {
            gather.get_mut(self.heap).state = GatherState::Awaited(AwaitedGather {
                awaiter: frame.awaiter,
                pending_children: frame.pending_children,
                results: frame.results,
            });
            Poll::Pending
        }
    }

    /// Fails every frame left on the commit stack with `err`, innermost first.
    ///
    /// This is the unwind the recursive walk got from `?`: each level caches the
    /// error so re-awaits replay it, and drops the result slots and awaiter the
    /// frame was holding. The children it had already committed keep running and
    /// keep pointing at their now-`Failed` gather, as they do for a failure that
    /// arrives after the commit pass (see [`HeapReadMut::fail`]).
    fn unwind_gather_commits(&mut self, stack: Vec<GatherCommit>, err: &RunError) {
        for frame in stack.into_iter().rev() {
            let HeapReadOutput::GatherFuture(gather) = self.heap.read(frame.gather) else {
                panic!("gather commit frame id is not a GatherFuture")
            };
            gather.get_mut(self.heap).state = GatherState::Failed(err.clone_with_heap(self.heap));
            drop(gather);

            frame.drop_with(self.heap);
        }
    }

    /// Awaits an external future by inspecting its heap state.
    ///
    /// - `Resolved(v)` returns a clone of `v` immediately.
    /// - `Failed(e)` re-raises a clone of `e`.
    /// - `Pending { awaiter: None }` installs the current task as the awaiter,
    ///   and returns `Poll::Pending`.
    /// - `Pending { awaiter: Some(_) }` is rejected as a double-await — a
    ///   single-awaiter restriction we keep until multi-awaiter wake/raise
    ///   plumbing lands.
    pub(super) fn await_external_future(
        &mut self,
        fut: &HeapObjectRead<'h, ExternalFuture>,
        awaiter: Awaiter,
    ) -> Result<Poll<Value>, RunError> {
        let mut awaiter_guard = DropGuard::new(awaiter, self);
        let (awaiter, this) = awaiter_guard.as_parts_mut();
        if fut.get(this.heap).task_group.as_ref().is_some_and(|group| {
            group.state != TaskGroupState::Exiting && group.state != TaskGroupState::Finished
                || !matches!(awaiter, Awaiter::Task(task) if Some(*task) == group.owner)
        }) {
            return Err(ExcType::type_error(
                "TaskGroup is an async context manager, not an awaitable",
            ));
        }
        if fut.get(this.heap).public_task.is_some() {
            this.prepare_public_join(fut.id(), awaiter)?;
            if matches!(fut.get(this.heap).state, ExternalFutureState::Pending { .. }) {
                let awaiter = awaiter_guard.into_inner();
                fut.get_mut(self.heap)
                    .public_task
                    .as_mut()
                    .expect("public task")
                    .waiters
                    .push(awaiter);
                return Ok(Poll::Pending);
            }
        }
        this.prepare_queue_operation(fut.id(), awaiter)?;
        this.prepare_child_join(fut.id(), awaiter)?;
        match &fut.get(this.heap).state {
            ExternalFutureState::Resolved(value) => {
                let value = value.clone_with_heap(this);
                Ok(Poll::Ready(value))
            }
            ExternalFutureState::Failed(_) => {
                // Older snapshots can contain a metadata-only cached failure.
                let ExternalFutureState::Failed(mut error) = mem::replace(
                    &mut fut.get_mut(this.heap).state,
                    ExternalFutureState::Pending { awaiter: None },
                ) else {
                    unreachable!("failed future")
                };
                error.materialize_object(this.heap);
                let raised = error.clone_with_heap(this.heap);
                fut.get_mut(this.heap).state = ExternalFutureState::Failed(error);
                Err(raised)
            }
            ExternalFutureState::Pending { awaiter: Some(_) } => {
                Err(SimpleException::new_msg(ExcType::RuntimeError, "cannot reuse already awaited future").into())
            }
            ExternalFutureState::Pending { awaiter: None } => {
                let awaiter = awaiter_guard.into_inner();
                fut.get_mut(self.heap).state = ExternalFutureState::Pending { awaiter: Some(awaiter) };
                Ok(Poll::Pending)
            }
        }
    }

    /// Starts execution of a coroutine by pushing its locals onto the stack.
    ///
    /// Extends the VM stack with the coroutine's pre-bound namespace values
    /// and pushes a new frame to execute the coroutine's function body.
    fn start_coroutine_frame(
        &mut self,
        func_id: FunctionId,
        namespace_values: Vec<Value>,
        globals: Option<HeapId>,
    ) -> Result<(), RunError> {
        let call_offset = self.current_offset();
        let code = &self.interns.get_function(func_id).code;
        let locals_count = u16::try_from(namespace_values.len()).expect("coroutine namespace size exceeds u16");

        // Extend the stack with the coroutine's pre-bound locals.
        let stack_base = self.stack.len();
        self.stack.extend(namespace_values);

        // Push frame to execute the coroutine
        let exc_stack_base = self.exception_stack.len();
        let namespace = function_namespace(globals, &*self.heap);
        self.push_frame(CallFrame::new_function(
            code,
            stack_base,
            locals_count,
            exc_stack_base,
            func_id,
            call_offset,
            namespace,
        ))?;

        Ok(())
    }

    /// Selects another task after blocking, or yields to the host with the current context intact.
    pub(super) fn switch_or_yield(&mut self) -> Result<AwaitResult, RunError> {
        if let Some(next_task_id) = self.scheduler.next_ready_task() {
            self.activate_task(next_task_id)?;
            Ok(AwaitResult::FramePushed)
        } else {
            Ok(AwaitResult::Yield(self.scheduler.pending_call_ids()))
        }
    }

    /// Removes a completed child and delivers its result, discarding it if its gather already settled.
    /// Resumes the gather's waiter when available, otherwise the next queued task.
    pub(super) fn handle_task_completion(&mut self, result: Value) -> Result<AwaitResult, RunError> {
        let task_id = self
            .scheduler
            .current_task_id()
            .expect("handle_task_completion called without current task");
        if self.is_public_task(task_id) {
            return self.complete_public_task(Ok(result));
        }
        if self.scheduler.get_task_mut(task_id).child_result.is_some() {
            return self.complete_owned_child(Ok(result));
        }
        // The awaiter keeps the gather alive across task teardown.
        let task = self.scheduler.get_task_mut(task_id);
        let awaiter = task.awaiter.take();
        let coroutine_id = task
            .coroutine_id
            .expect("handle_task_completion: spawned task without a coroutine");

        // Re-awaiting the coroutine must see that its execution has ended.
        let HeapReadOutput::Coroutine(coro) = self.heap.read(coroutine_id) else {
            panic!("task coroutine_id doesn't point to a Coroutine")
        };
        coro.get_mut(self.heap).state = CoroutineState::Completed;
        drop(coro);

        let group = self.scheduler.get_task_mut(task_id).group_parent;
        let parent = self.scheduler.transfer_gather_lifetime(task_id);
        self.scheduler.cancel_task(task_id, self.heap);
        if let Some(group) = group {
            self.task_group_child_completed(group, false);
        }

        let delivery = if let Some(awaiter) = awaiter {
            self.deliver_awaiter_success(awaiter, result)
        } else {
            result.drop_with(self);
            None
        };

        self.cleanup_current_task();

        if let Some(parent) = parent {
            self.finish_child_drain(parent);
        }

        if self.resume_after_task_exit(delivery)? {
            Ok(AwaitResult::FramePushed)
        } else {
            Ok(AwaitResult::Yield(self.scheduler.pending_call_ids()))
        }
    }

    /// Returns true if the current task is a spawned task (not main).
    ///
    /// Used by exception handling to determine if an unhandled exception
    /// should fail the task rather than propagate out.
    #[inline]
    pub(super) fn is_spawned_task(&self) -> bool {
        self.scheduler.current_task_id().is_some_and(|id| !id.is_main())
    }

    /// Removes a child whose exception escaped its frames and notifies its gather.
    /// A settled gather discards the error. Any returned error belongs to the newly active task
    /// and must go through exception handling before that task executes bytecode.
    pub(super) fn handle_task_failure(&mut self, mut error: RunError) -> Result<(), RunError> {
        let task_id = self
            .scheduler
            .current_task_id()
            .expect("handle_task_failure called without current task");
        debug_assert!(!task_id.is_main(), "handle_task_failure called for main task");

        if self.is_public_task(task_id) {
            self.complete_public_task(Err(error))?;
            return Ok(());
        }
        if self.scheduler.get_task_mut(task_id).child_result.is_some() {
            self.complete_owned_child(Err(error))?;
            return Ok(());
        }
        error.materialize_object(self.heap);
        let retained = self.unreceived_gather_error(task_id, &error);
        let awaiter = self.scheduler.get_task_mut(task_id).awaiter.take();
        let delivery = if let Some(awaiter) = awaiter {
            self.deliver_awaiter_failure(awaiter, error)
        } else {
            error.drop_with(self);
            None
        };
        if delivery.is_none() {
            if let Some(group) = self.scheduler.get_task_mut(task_id).group_parent
                && let Some(value) = &retained
            {
                let value = value.clone_with_heap(self.heap);
                let HeapReadOutput::ExternalFuture(group) = self.heap.read(group) else {
                    unreachable!()
                };
                group
                    .get_mut(self.heap)
                    .task_group
                    .as_mut()
                    .expect("group")
                    .errors
                    .push(value);
            }
            self.scheduler.get_task_mut(task_id).gather_errors.extend(retained);
        } else {
            retained.drop_with(self);
        }
        let group = self.scheduler.get_task_mut(task_id).group_parent;
        let parent = self.scheduler.transfer_gather_lifetime(task_id);
        self.cleanup_current_task();
        self.scheduler.cancel_task(task_id, self.heap);
        if let Some(group) = group {
            self.task_group_child_completed(group, false);
        }
        if let Some(parent) = parent {
            self.finish_child_drain(parent);
        }
        self.resume_after_task_exit(delivery)?;
        Ok(())
    }

    /// Cancellation identity, rather than its message, distinguishes an expected
    /// signal from an ordinary cleanup error that happens to use the same text.
    fn unreceived_gather_error(&self, task_id: TaskId, error: &RunError) -> Option<Value> {
        let task = self.scheduler.task(task_id)?;
        task.lifetime_parent?;
        let RunError::Exc(error) = error else { return None };
        let value = error.object.as_ref()?;
        if matches!((&task.child_cancel_signal, value), (Some(Value::Ref(a)), Value::Ref(b)) if a == b) {
            None
        } else {
            Some(value.clone_with_heap(self.heap))
        }
    }

    /// Selects the task awakened by a child's exit, or the next queued task if none was awakened.
    /// The exiting task and its VM context must already be discarded. Returns false if all tasks block.
    pub(super) fn resume_after_task_exit(&mut self, waiter: Option<TaskId>) -> RunResult<bool> {
        let next = if let Some(waiter) = waiter {
            self.scheduler.remove_from_ready_queue(waiter);
            Some(waiter)
        } else {
            self.scheduler.next_ready_task()
        };
        if let Some(task_id) = next {
            self.activate_task(task_id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Saves the current VM context into the given task in the scheduler.
    ///
    /// Serializes frames, moves stack/exception_stack, stores instruction_ip,
    /// and adjusts the global recursion depth counter.
    pub(super) fn save_task_context(&mut self, task_id: TaskId) {
        let barrier = self
            .scheduler
            .task(task_id)
            .is_some_and(|task| task.root_shutdown.is_some() || task.cleanup_shutdown.is_some());
        let protected = barrier
            || self
                .suspended_frames
                .iter()
                .chain([&self.current_frame])
                .any(|f| f.code.native_cleanup_at(f.ip.saturating_sub(1)));
        self.scheduler.get_task_mut(task_id).native_protected = protected;
        let mut frames: Vec<SerializedTaskFrame> = self
            .suspended_frames
            .drain(..)
            .map(|f| SerializedTaskFrame {
                function_id: f.function_id,
                ip: f.ip,
                stack_base: f.stack_base(),
                locals_count: f.locals_count,
                exception_stack_base: f.exception_stack_base(),
                call_offset: f.call_offset,
                is_initializer: f.is_initializer,
                namespace: f.namespace,
                generator_activation: f.generator_activation,
                import_from_module: f.import_from_module,
            })
            .collect();
        // The namespace moves across so the frame left behind releases nothing.
        let current = &mut self.current_frame;
        frames.push(SerializedTaskFrame {
            function_id: current.function_id,
            ip: current.ip,
            stack_base: current.stack_base(),
            locals_count: current.locals_count,
            exception_stack_base: current.exception_stack_base(),
            call_offset: current.call_offset,
            is_initializer: current.is_initializer,
            namespace: mem::take(&mut current.namespace),
            generator_activation: current.generator_activation.take(),
            import_from_module: current.import_from_module,
        });

        // Count this task's recursion depth contribution and subtract it from
        // the global counter so the next task gets a clean budget.
        let task_depth = frame_recursion_depth(frames.iter().map(|f| f.function_id), self.interns);
        self.recursion_depth -= task_depth;

        // Save VM state into the task
        let task = self.scheduler.get_task_mut(task_id);
        task.frames = frames;
        task.stack = mem::take(&mut self.stack);
        task.exception_stack = mem::take(&mut self.exception_stack);
        task.instruction_ip = self.instruction_ip;
    }

    /// Activates a task whose queue entry was removed, taking its resume result exactly once.
    /// Any returned error must be raised in the loaded context before executing bytecode.
    pub(super) fn activate_task(&mut self, task_id: TaskId) -> RunResult<()> {
        let result = self.scheduler.take_resume_result(task_id);
        if self.scheduler.current_task_id() != Some(task_id) {
            if let Some(current) = self.scheduler.current_task_id() {
                self.save_task_context(current);
            }
            self.scheduler.set_current_task(Some(task_id));
            self.load_or_init_task(task_id)?;
        }
        // Failed first activation may already have removed this task and selected another.
        if self.scheduler.current_task_id() != Some(task_id) {
            return result;
        }
        let gathered =
            mem::take(&mut self.scheduler.get_task_mut(task_id).pending_gather_error) == GatherErrorDelivery::Pending;
        // A cancelled ready child must not start more body work. Protected
        // cleanup keeps the queued value/error and delivers at its continuation.
        if self.scheduler.current_task().is_some_and(|task| {
            !task.native_protected
                && !task.gather_protected
                && task.native_cancel.as_ref().is_some_and(|cancel| !cancel.delivered)
        }) {
            if let Err(error) = result {
                if gathered
                    && let RunError::Exc(exc) = &error
                    && let Some(value) = &exc.object
                {
                    self.scheduler
                        .get_task_mut(task_id)
                        .gather_errors
                        .push(value.clone_with_heap(self.heap));
                }
                error.drop_with(self);
            }
            self.scheduler
                .get_task_mut(task_id)
                .pending_join
                .take()
                .map(Value::Ref)
                .drop_with(self.heap);
            self.cancel_queue_operation(task_id);
            return Err(self.native_delivery(None));
        }
        self.consume_queue_delivery(task_id)?;
        self.consume_child_delivery(task_id);
        result
    }

    /// Restores a task's saved context and recursion depth, or starts its coroutine on first use.
    /// Resolved values are already on the saved stack. Pending exceptions are handled by `activate_task`.
    fn load_or_init_task(&mut self, task_id: TaskId) -> Result<(), RunError> {
        let task = self.scheduler.get_task_mut(task_id);
        let frames = mem::take(&mut task.frames);
        let stack = mem::take(&mut task.stack);
        let exception_stack = mem::take(&mut task.exception_stack);
        let instruction_ip = task.instruction_ip;
        let coroutine_id = task.coroutine_id;

        // Restore this task's recursion depth contribution to the global counter
        let task_depth = frame_recursion_depth(frames.iter().map(|f| f.function_id), self.interns);
        self.recursion_depth += task_depth;

        if !frames.is_empty() {
            // Task has existing context - restore it
            self.stack = stack;
            self.exception_stack = exception_stack;
            self.instruction_ip = instruction_ip;

            // Reconstruct the suspended callers and current frame.
            let mut frames: Vec<_> = frames
                .into_iter()
                .map(|sf| {
                    let code = frame_code(self.interns, self.module_code, sf.function_id);
                    CallFrame {
                        code,
                        bytecode: code.bytecode(),
                        ip: sf.ip,
                        stack_base: stack_index(sf.stack_base),
                        locals_count: sf.locals_count,
                        exception_stack_base: stack_index(sf.exception_stack_base),
                        function_id: sf.function_id,
                        call_offset: sf.call_offset,
                        should_return: false,
                        is_parked: false,
                        namespace: sf.namespace,
                        generator_activation: sf.generator_activation,
                        is_initializer: sf.is_initializer,
                        import_from_module: sf.import_from_module,
                    }
                })
                .collect();
            self.current_frame = frames.pop().expect("task context contains no active frame");
            self.suspended_frames = frames;
        } else if let Some(coro_id) = coroutine_id {
            // The previous task's frames are already saved. A reused coroutine
            // must fail this new task and select its successor before the
            // exception can propagate.
            let HeapReadOutput::Coroutine(coro) = self.heap.read(coro_id) else {
                panic!("task coroutine_id doesn't point to a Coroutine")
            };
            let is_new = coro.get(self.heap).state == CoroutineState::New;
            // Release the handle before either branch: both go on to drop
            // references to this coroutine, and freeing it under a live
            // reader panics.
            drop(coro);
            if is_new {
                self.init_task_from_coroutine(coro_id)?;
            } else {
                return self.handle_task_failure(ExcType::cannot_reuse_already_awaited_coroutine());
            }
        } else {
            // This shouldn't happen - task with no frames and no coroutine
            panic!("task has no frames and no coroutine_id");
        }

        Ok(())
    }

    /// Initializes the VM state to run a coroutine for a spawned task.
    ///
    /// Similar to exec_get_awaitable's coroutine handling, but for task initialization.
    fn init_task_from_coroutine(&mut self, coroutine_id: HeapId) -> Result<(), RunError> {
        let HeapReadOutput::Coroutine(coro) = self.heap.read(coroutine_id) else {
            panic!("task coroutine_id doesn't point to a Coroutine")
        };

        // Check state
        if coro.get(self.heap).state != CoroutineState::New {
            return Err(ExcType::cannot_reuse_already_awaited_coroutine());
        }

        // Extract coroutine data
        let func_id = coro.get(self.heap).func_id;
        let globals = coro.get(self.heap).globals;
        let namespace_values: Vec<Value> = coro
            .get(self.heap)
            .namespace
            .iter()
            .map(|v| v.clone_with_heap(self))
            .collect();

        // Mark coroutine as Running
        coro.get_mut(self.heap).state = CoroutineState::Running;

        // Push locals onto stack and push frame directly (can't use start_coroutine_frame
        // because that needs a current frame for call_offset, but spawned tasks
        // don't have a parent frame — the coroutine is the root)
        let code = &self.interns.get_function(func_id).code;
        let locals_count = u16::try_from(namespace_values.len()).expect("coroutine namespace size exceeds u16");

        let stack_base = self.stack.len();
        self.stack.extend(namespace_values);

        let exc_stack_base = self.exception_stack.len();
        let namespace = function_namespace(globals, &*self.heap);
        self.current_frame = CallFrame::new_function(
            code,
            stack_base,
            locals_count,
            exc_stack_base,
            func_id,
            None, // No call position — this is the root frame for a spawned task
            namespace,
        );
        self.suspended_frames.clear();

        Ok(())
    }

    /// Resolves an external future with a value.
    ///
    /// Called by the host when an async external call completes. Looks up
    /// the `ExternalFuture` heap entry for `call_id`, transitions it to
    /// `Resolved(value)`, and delivers `value` to the awaiter (if any).
    pub fn resolve_future(&mut self, call_id: u32, value: Value) {
        let call_id = CallId::new(call_id);

        let Some(future_id) = self.scheduler.take_pending_external(call_id) else {
            value.drop_with(self);
            return;
        };

        // Ensure future cleaned up on all paths
        let fut_val = Value::Ref(future_id);
        let this = self;
        defer_drop!(fut_val, this);

        let HeapReadOutput::ExternalFuture(fut) = this.heap.read(future_id) else {
            panic!("pending_externals entry doesn't point to an ExternalFuture")
        };

        // `asyncio.sleep(delay, result)`: the host's value was only the
        // wake-up signal, `result` is what the await produces.
        let value = match fut.get_mut(this.heap).sleep_result.take() {
            Some(result) => {
                value.drop_with(this);
                result
            }
            None => value,
        };
        let mut value_guard = DropGuard::new(value, this);
        let (value, this) = value_guard.as_parts_mut();

        let awaiter_and_value = match &mut fut.get_mut(this.heap).state {
            ExternalFutureState::Pending { awaiter } => awaiter.take().map(|a| (a, value.clone_with_heap(this.heap))),
            ExternalFutureState::Resolved(_) | ExternalFutureState::Failed(_) => {
                panic!("resolve_future: future was already resolved")
            }
        };

        let (value, this) = value_guard.into_parts();
        fut.get_mut(this.heap).state = ExternalFutureState::Resolved(value);

        if let Some((awaiter, value)) = awaiter_and_value {
            this.deliver_awaiter_success(awaiter, value);
        }
        drop(fut);
        if let Some(owner) = this.scheduler.gather_externals.remove(&call_id) {
            this.scheduler
                .get_task_mut(owner)
                .cleanup_externals
                .retain(|(call, _)| *call != call_id);
            this.finish_child_drain(owner);
        }
    }

    /// Pushes `value` onto a blocked task's stack and queues it, returning the awakened task.
    /// Drops the value if the task is gone or its awaitable has already settled.
    fn deliver_value_to_task(&mut self, task_id: TaskId, value: Value) -> Option<TaskId> {
        if !self.scheduler.is_blocked(task_id) {
            value.drop_with(self);
            return None;
        }

        let task_is_current = self.scheduler.current_task_id() == Some(task_id) && !self.current_frame.is_parked;
        if task_is_current {
            self.stack.push(value);
        } else {
            self.scheduler.get_task_mut(task_id).stack.push(value);
        }
        self.scheduler.make_ready(task_id, Ok(()), self.heap);
        Some(task_id)
    }

    /// Delivers a value through nested gathers, queueing the terminal task if the chain completes.
    /// Returns the awakened task, or `None` if a gather is pending, a gather already settled,
    /// or the terminal task no longer awaits a result.
    pub(super) fn deliver_awaiter_success(&mut self, mut awaiter: Awaiter, mut value: Value) -> Option<TaskId> {
        let this = self;
        loop {
            match awaiter {
                Awaiter::Task(t) => return this.deliver_value_to_task(t, value),
                Awaiter::GatherSlot { gather, source } => {
                    let gather_val = Value::Ref(gather);
                    defer_drop!(gather_val, this);
                    let HeapReadOutput::GatherFuture(outer) = this.heap.read(gather) else {
                        panic!("Awaiter::GatherSlot gather id is not a GatherFuture")
                    };
                    let success = outer.resolve_child(this, source, value)?;
                    awaiter = success.awaiter;
                    value = Value::Ref(success.list_id);
                }
            }
        }
    }

    /// Fails each gather in the awaiter chain and queues its terminal task to raise `error`.
    /// Returns that task's ID, or `None` if the task is gone or a gather already settled.
    /// The task raises the exception on activation; its own gather is notified only if it escapes.
    pub(super) fn deliver_awaiter_failure(&mut self, awaiter: Awaiter, mut error: RunError) -> Option<TaskId> {
        let gathered = matches!(awaiter, Awaiter::GatherSlot { .. });
        let this = self;
        defer_drop_mut!(awaiter, this);
        error.materialize_object(this.heap);
        let target = loop {
            let next = match awaiter {
                Awaiter::Task(t) => break *t,
                Awaiter::GatherSlot { gather, .. } => {
                    let HeapReadOutput::GatherFuture(gather) = this.heap.read(*gather) else {
                        panic!("Awaiter::GatherSlot gather id is not a GatherFuture")
                    };
                    // A gather that already settled has handed its waiter on;
                    // this failure arrives after the fact and stops here.
                    let Some(next) = gather.fail(this.heap, &error) else {
                        drop(gather);
                        error.drop_with(this);
                        return None;
                    };
                    next
                }
            };
            mem::replace(awaiter, next).drop_with(this);
        };
        if !this.scheduler.is_blocked(target) {
            error.drop_with(this);
            return None;
        }
        this.scheduler.get_task_mut(target).pending_gather_error = if gathered {
            GatherErrorDelivery::Pending
        } else {
            GatherErrorDelivery::None
        };
        this.scheduler.make_ready(target, Err(error), this.heap);
        // Protected ordinary cleanup cannot continue to parent release while a
        // failed gather still owns children. Keep the first error in Ready (so
        // late settlements cannot overwrite it), but withhold its runnable slot.
        // Explicit experimental scopes retain their separate drain/error policy.
        if gathered && this.hold_protected_gather_error(target) {
            return None;
        }
        Some(target)
    }

    /// A Ready(error) owns the retained first error; only its runnable slot is
    /// withheld. Lifetime completion, including transferred descendants, releases
    /// the slot. Late gather settlements cannot overwrite this ready state.
    fn hold_protected_gather_error(&mut self, target: TaskId) -> bool {
        let external = self.scheduler.gather_externals.values().any(|owner| *owner == target);
        let public = self
            .scheduler
            .public_gather_waits
            .get(&target)
            .is_some_and(|waits| !waits.is_empty());
        let task = self.scheduler.get_task_mut(target);
        if (task.native_protected || task.gather_protected)
            && task.scope_token.is_none()
            && (!task.gather_children.is_empty() || external || public)
        {
            task.pending_gather_error = GatherErrorDelivery::Draining;
            self.scheduler.remove_from_ready_queue(target);
            true
        } else {
            false
        }
    }

    /// Records a host rejection and queues the task that must raise it.
    /// Task selection waits until the whole batch has been ingested.
    pub fn fail_future(&mut self, call_id: u32, mut error: RunError) {
        let Some(future_id) = self.scheduler.take_pending_external(CallId::new(call_id)) else {
            error.drop_with(self);
            return;
        };
        let fut_val = Value::Ref(future_id);
        let this = self;
        defer_drop!(fut_val, this);

        let HeapReadOutput::ExternalFuture(fut) = this.heap.read(future_id) else {
            panic!("pending_externals entry doesn't point to an ExternalFuture")
        };
        error.materialize_object(this.heap);
        let owner = this.scheduler.gather_externals.remove(&CallId::new(call_id));
        if let Some(owner) = owner {
            this.scheduler
                .get_task_mut(owner)
                .cleanup_externals
                .retain(|(call, _)| *call != CallId::new(call_id));
        }
        let retained = if owner.is_some() {
            if let RunError::Exc(exc) = &error {
                exc.object.as_ref().map(|v| v.clone_with_heap(this.heap))
            } else {
                None
            }
        } else {
            None
        };
        let cached = error.clone_with_heap(this.heap);
        let awaiter = match mem::replace(&mut fut.get_mut(this.heap).state, ExternalFutureState::Failed(cached)) {
            ExternalFutureState::Pending { awaiter } => awaiter,
            ExternalFutureState::Resolved(_) | ExternalFutureState::Failed(_) => {
                panic!("fail_future: future was already resolved")
            }
        };
        // A failed sleep never produces its result.
        let sleep_result = fut.get_mut(this.heap).sleep_result.take();
        drop(fut);
        sleep_result.drop_with(this);
        let delivery = if let Some(awaiter) = awaiter {
            this.deliver_awaiter_failure(awaiter, error)
        } else {
            error.drop_with(this);
            None
        };
        if let Some(owner) = owner {
            if delivery.is_none() {
                this.scheduler.get_task_mut(owner).gather_errors.extend(retained);
            } else {
                retained.drop_with(this);
            }
            this.finish_child_drain(owner);
        } else {
            retained.drop_with(this);
        }
    }

    /// Allocates an `ExternalFuture` for `call_id` and pushes a `Value::Ref`
    /// to it on the VM stack.
    ///
    /// The scheduler indexes `call_id -> future_id` (with its own inc_ref) so
    /// host resolutions can find the heap entry; the `Value::Ref` pushed onto
    /// the stack is the user's reference, which travels with the value until
    /// it's awaited or dropped.
    pub fn add_pending_call(&mut self, call_id: CallId) {
        self.push_pending_future(call_id, None);
    }

    /// Like [`Self::add_pending_call`], for a host answering `asyncio.sleep`
    /// with a future: the awaitable resolves with `result`, the value the
    /// call was given, rather than with whatever the host resolves it with.
    pub(crate) fn add_pending_sleep(&mut self, call_id: CallId, result: Value) {
        self.push_pending_future(call_id, Some(result));
    }

    /// Allocates the pending `ExternalFuture`, indexes it for the host's
    /// resolution and pushes it as the call's value.
    fn push_pending_future(&mut self, call_id: CallId, sleep_result: Option<Value>) {
        let future_id = self
            .heap
            .allocate(HeapData::ExternalFuture(Box::new(ExternalFuture::new_pending(
                call_id,
                sleep_result,
            ))));
        self.scheduler.add_pending_external(call_id, future_id, self.heap);
        self.push(Value::Ref(future_id));
    }

    /// Allocates an `ExternalFuture` already resolved with `value`, for an OS
    /// call whose result the sandbox awaits (`asyncio.sleep`).
    ///
    /// The host answered immediately, so nothing is registered with the
    /// scheduler — awaiting the future hands the value straight back, and
    /// dropping it unawaited releases the value like any other awaitable. The
    /// call id is allocated rather than reused so it stays unique if the
    /// future is ever inspected.
    pub(crate) fn settled_awaitable(&mut self, value: Value) -> Value {
        let call_id = self.allocate_call_id();
        let future = ExternalFuture {
            public_task: None,
            task_group: None,
            call_id,
            state: ExternalFutureState::Resolved(value),
            sleep_result: None,
            child: None,
            queue: None,
            queue_op: None,
        };
        Value::Ref(self.heap.allocate(HeapData::ExternalFuture(Box::new(future))))
    }

    /// Raises `exc` uncatchably at the suspension point, for hosts enforcing
    /// a limit while execution is suspended.
    ///
    /// A `ResolveFutures` snapshot is parked when the last runnable task
    /// finished with the rest still blocked, so the VM holds a placeholder
    /// frame and every real frame lives in the scheduler. Raising there would
    /// point the traceback at line 1, so the main task is reloaded first: it
    /// is always alive while futures are pending, and its `await` is the
    /// suspension the user sees.
    pub fn abort(&mut self, exc: MontyException) -> RunResult<FrameExit> {
        let main = TaskId::default();
        if self.current_frame.is_parked && self.scheduler.main_task().is_some_and(|task| !task.frames.is_empty()) {
            self.scheduler.set_current_task(Some(main));
            self.load_or_init_task(main)?;
        }
        self.resume_with_exception(RunError::uncatchable(exc))
    }

    /// Caches host results and wakes their awaiters without executing bytecode.
    /// Eager results have no awaiter yet; their following `await` reads the cached result.
    pub fn apply_future_results(&mut self, results: Vec<(u32, ExtFunctionResult)>) -> RunResult<()> {
        for (call_id, ext_result) in results {
            // First settlement wins, including within one batch and after revocation.
            // Ignore before value import so stale replies cannot abort protected cleanup.
            if !self.scheduler.has_pending_call(call_id) {
                continue;
            }
            match ext_result {
                ExtFunctionResult::Return(obj) => {
                    // A resource error is a `MemoryError` here as in `resume`,
                    // so a large host value is not misreported as a bad type.
                    let value = obj.to_value(self).map_err(|e| match e {
                        InvalidInputError::Resource(err) => RunError::from(err),
                        other @ InvalidInputError::InvalidType(_) => RunError::from(MontyException::runtime_error(
                            format!("Invalid return value for call {call_id}: {other}"),
                        )),
                    })?;
                    self.resolve_future(call_id, value);
                }
                ExtFunctionResult::Error(exc) => self.fail_future(call_id, RunError::from(exc)),
                ExtFunctionResult::Future(_) => {}
                ExtFunctionResult::NotFound(function_name) => {
                    self.fail_future(call_id, ExtFunctionResult::not_found_exc(&function_name));
                }
            }
        }
        Ok(())
    }

    /// Host-facing terminal projection reads the native owner, never guest labels.
    pub(crate) fn native_cancel_reason(&self) -> Option<&str> {
        self.scheduler
            .main_task()?
            .native_cancel
            .as_ref()
            .map(|c| c.reason.as_str())
    }

    /// First-cause cancellation for the native owner and its scoped descendants.
    /// A protected wait keeps its legitimate reply. Delivery consumes the original
    /// external registration just like failure, so a late result cannot revive it.
    pub(crate) fn cancel_native(&mut self, reason: &str) -> RunResult<()> {
        // Repeats retain ownership after the native root returns to its caller.
        if self.scheduler.main_task().is_some_and(|t| t.native_cancel.is_some()) {
            return self.deliver_native_cancel();
        }
        self.check_native_cancel_code()?;
        let native_generator = self
            .suspended_frames
            .iter()
            .chain([&self.current_frame])
            .any(|f| f.generator_activation.as_ref().is_some_and(|a| a.native))
            || self.scheduler.main_task().is_some_and(|task| {
                task.frames
                    .iter()
                    .any(|f| f.generator_activation.as_ref().is_some_and(|a| a.native))
            });
        let coroutine = self
            .suspended_frames
            .iter()
            .chain([&self.current_frame])
            .any(|f| self.is_coroutine_frame(f.function_id))
            || self
                .scheduler
                .main_task()
                .is_some_and(|task| task.frames.iter().any(|f| self.is_coroutine_frame(f.function_id)));
        if !native_generator && !coroutine {
            return Err(MontyException::runtime_error("native cancellation requires a suspended native task").into());
        }
        self.scheduler.adopt_root_tasks();
        self.cancel_public_task_reason(TaskId::default(), reason);
        self.deliver_native_cancel()
    }

    /// Function identity survives task parking and snapshot restoration. Module-level
    /// await and ordinary generators retain their separate execution policies.
    pub(super) fn is_coroutine_frame(&self, function: Option<FunctionId>) -> bool {
        function.is_some_and(|id| self.interns.get_function(id).is_async)
    }

    /// Older genuine dumps remain usable for ordinary execution, but cannot
    /// resume a cancellation contract whose continuation points they lack.
    fn check_native_cancel_code(&self) -> RunResult<()> {
        let legacy = self
            .suspended_frames
            .iter()
            .chain([&self.current_frame])
            .any(|f| !f.code.supports_native_cancel())
            || self.scheduler.owned_task_tree(self.heap).iter().any(|id| {
                self.scheduler.task(*id).is_some_and(|task| {
                    task.frames.iter().any(|f| {
                        let code = frame_code(self.interns, self.module_code, f.function_id);
                        !code.supports_native_cancel()
                    })
                })
            });
        if legacy {
            Err(RunError::uncatchable(MontyException::runtime_error(
                "native cancellation requires catchable cancellation policy 7",
            )))
        } else {
            Ok(())
        }
    }

    /// A compiler-owned continuation boundary, after operand/exception cleanup.
    /// The ordinary error path owns propagation, stack truncation and frame release.
    /// A pending return is superseded by this signal; escaping errors arbitrate
    /// separately at the remaining handler-search site. Active caller cleanup masks helper checkpoints.
    pub(super) fn native_cleanup_continuation(&mut self, returning: bool) -> RunResult<()> {
        if self.scheduler.current_task().is_some_and(|task| task.gather_protected) {
            return Ok(());
        }
        if let Some(error) = self.pending_group_interrupt() {
            return Err(error);
        }
        let Some(cancel) = self
            .scheduler
            .current_task()
            .and_then(|task| task.native_cancel.as_ref())
        else {
            return Ok(());
        };
        if cancel.delivered {
            return Ok(());
        }
        if !returning
            && self
                .current_frame
                .code
                .native_cancel_points
                .as_ref()
                .is_none_or(|points| !points.contains(&u32::try_from(self.instruction_ip).expect("instruction offset")))
        {
            return Ok(());
        }
        if (!returning && self.current_frame.code.native_cleanup_at(self.instruction_ip))
            || self
                .suspended_frames
                .iter()
                .any(|frame| frame.code.native_cleanup_at(frame.ip.saturating_sub(1)))
        {
            return Ok(());
        }
        let diagnostic = self.native_matching_diagnostic();
        Err(self.native_delivery(diagnostic))
    }

    /// Active dispatch consults the VM's own paired frames and exception stack.
    fn native_matching_diagnostic(&self) -> Option<super::scheduler::InterruptedError> {
        let index = self
            .current_frame
            .code
            .native_matching_at(self.instruction_ip)
            .map(|depth| self.current_frame.exception_stack_base() + depth)
            .or_else(|| {
                self.suspended_frames.iter().rev().find_map(|f| {
                    f.code
                        .native_matching_at(f.ip.saturating_sub(1))
                        .map(|depth| f.exception_stack_base() + depth)
                })
            })?;
        self.native_caught_diagnostic(&self.exception_stack[index])
    }

    fn native_caught_diagnostic(
        &self,
        caught: &super::exceptions::CaughtException,
    ) -> Option<super::scheduler::InterruptedError> {
        let Value::Ref(id) = caught.value() else {
            return None;
        };
        let HeapData::Exception(exc) = self.heap.get(*id) else {
            return None;
        };
        Some(super::scheduler::InterruptedError::capture(&exc.simple))
    }

    /// Freeze before routing reentry, including a final null diagnostic.
    pub(super) fn native_delivery(&mut self, diagnostic: Option<super::scheduler::InterruptedError>) -> RunError {
        // Host cancellation can target the saved main task while the VM is parked.
        let task_id = self.scheduler.current_task_id().unwrap_or_default();
        let cancel = self.scheduler.get_task_mut(task_id).native_cancel.as_mut().unwrap();
        debug_assert!(!cancel.delivered);
        cancel.interrupted_error = diagnostic;
        cancel.delivered = true;
        let mut signal: RunError = MontyException::new(ExcType::ValueError, Some(cancel.reason.clone())).into();
        // Root groups also need the exact cancellation identity to exclude
        // their own delivery from ordinary body failure aggregation.
        signal.materialize_object(self.heap);
        if let RunError::Exc(error) = &signal {
            self.scheduler.get_task_mut(task_id).child_cancel_signal =
                error.object.as_ref().map(|value| value.clone_with_heap(self.heap));
        }
        signal
    }

    pub(crate) fn native_interrupted_error(&self) -> Option<super::scheduler::InterruptedError> {
        self.scheduler
            .main_task()?
            .native_cancel
            .as_ref()?
            .interrupted_error
            .clone()
    }

    /// Pending-only admission must run before frame installation/state transfer.
    pub(super) fn admit_native_code(&self, code: &Code) -> RunResult<()> {
        if self
            .scheduler
            .main_task()
            .and_then(|t| t.native_cancel.as_ref())
            .is_some_and(|c| !c.delivered)
            && !code.supports_native_cancel()
        {
            return Err(RunError::uncatchable(MontyException::runtime_error(
                "native cancellation requires catchable cancellation policy 7",
            )));
        }
        Ok(())
    }

    /// Interrupt active selection at its continuation; local catchers may recover.
    pub(super) fn native_matching_error(&mut self, returning: bool) -> Option<RunError> {
        if self.scheduler.current_task()?.gather_protected {
            return None;
        }
        let cancel = self.scheduler.current_task()?.native_cancel.as_ref()?;
        if cancel.delivered
            || (!returning && self.current_frame.code.native_cleanup_at(self.instruction_ip))
            || self
                .suspended_frames
                .iter()
                .any(|f| f.code.native_cleanup_at(f.ip.saturating_sub(1)))
        {
            return None;
        }
        let diagnostic = self.native_matching_diagnostic()?;
        Some(self.native_delivery(Some(diagnostic)))
    }

    /// Saved frames belong to the target task's exception stack, not the parked VM.
    pub(super) fn task_cancel_diagnostic(&self, id: TaskId) -> Option<super::scheduler::InterruptedError> {
        let task = self.scheduler.task(id)?;
        if task.frames.is_empty() {
            return self.native_matching_diagnostic();
        }
        task.frames.iter().enumerate().rev().find_map(|(i, f)| {
            let ip = if i + 1 == task.frames.len() {
                task.instruction_ip
            } else {
                f.ip.saturating_sub(1)
            };
            let code = frame_code(self.interns, self.module_code, f.function_id);
            code.native_matching_at(ip)
                .and_then(|depth| self.native_caught_diagnostic(&task.exception_stack[f.exception_stack_base + depth]))
        })
    }

    /// Repeat requests never reinject or consume a protected cleanup reply.
    fn deliver_native_cancel(&mut self) -> RunResult<()> {
        if let Some(cancel) = self.scheduler.main_task().and_then(|task| task.native_cancel.as_ref()) {
            if cancel.policy != 7 {
                return Err(RunError::uncatchable(MontyException::runtime_error(
                    "native cancellation requires catchable cancellation policy 7",
                )));
            }
            if !cancel.delivered {
                self.check_native_cancel_code()?;
            }
            let reason = cancel.reason.clone();
            for id in self.scheduler.owned_task_tree(self.heap) {
                self.cancel_child_task(id, &reason);
            }
        }
        Ok(())
    }

    /// Ingests a host batch, then resumes tasks in resolution order for both values and errors.
    /// With no runnable task, yields the remaining pending call IDs to the host.
    /// Returns an internal error if neither runnable tasks nor pending calls remain.
    pub fn resume_with_resolved_futures(&mut self, results: Vec<(u32, ExtFunctionResult)>) -> RunResult<FrameExit> {
        self.deliver_native_cancel()?;
        self.apply_future_results(results)?;
        if let Some(next_task_id) = self.scheduler.next_ready_task() {
            if let Err(error) = self.activate_task(next_task_id) {
                return self.resume_with_exception(error);
            }
            return self.run_external();
        }

        let pending_call_ids = self.scheduler.pending_call_ids();

        if pending_call_ids.is_empty() {
            // A stalled turn loses one `feed_run`, aborting loses the session.
            Err(RunError::internal(
                "asyncio scheduler stalled: no ready tasks and no pending external calls",
            ))
        } else {
            Ok(FrameExit::ResolveFutures(pending_call_ids))
        }
    }
}

/// One gather part-way through being committed — a frame of the walk in
/// [`VM::commit_gather_tree`], and the reason that walk needs no native stack.
///
/// Everything the recursive commit held in local variables lives here instead:
/// the items handled so far, and where in `items` to resume once the nested
/// gather this frame descended into has settled.
struct GatherCommit {
    /// The gather being committed. Borrowed — the reference is owned by the
    /// parent gather's `items`, or for the root by the awaited value itself.
    gather: HeapId,
    /// Owned awaiter, installed on the gather when it parks in `Awaited` and
    /// dropped when it settles synchronously. `Awaiter::GatherSlot` carries an
    /// inc_ref on the parent gather (see [`Awaiter`]).
    awaiter: Awaiter,
    /// Slot in the parent frame's `results` that this gather fills; `None` for
    /// the root, whose result goes to the `await` site instead.
    parent_slot: Option<usize>,
    /// Next index into the gather's `items` to commit.
    next: usize,
    /// Result slots, one per item, in `items` order. Owned values.
    results: Vec<Option<Value>>,
    /// Children committed but not yet settled → the slots they fill.
    pending_children: PendingChildren,
}

impl GatherCommit {
    /// Records the outcome of the nested gather this frame descended into and
    /// resumes at the following item.
    ///
    /// A nested gather that settled synchronously fills its slot like any other
    /// ready item; one that parked joins `pending_children`, so a later
    /// resolution reaches this gather through [`HeapReadMut::resolve_child`].
    fn resume_after_child(&mut self, child: HeapId, slot: usize, poll: Poll<Value>) {
        match poll {
            Poll::Ready(value) => self.results[slot] = Some(value),
            Poll::Pending => {
                self.pending_children.insert(child, smallvec![slot]);
            }
        }
        self.next = slot + 1;
    }
}

impl<C: ContainsHeap> DropWithContext<C> for GatherCommit {
    fn drop_with(self, heap: &mut C) {
        self.results.drop_with(heap);
        self.awaiter.drop_with(heap);
    }
}

/// Preflights the commit stack's next reallocation against `max_memory`.
///
/// A `Vec` grows by allocating the new buffer while the old one is still live,
/// so one growth part-way through a deep walk can add several MiB at once —
/// enough to clear the allocator's hard ceiling between two periodic checks,
/// which kills the worker rather than ending the run.
fn check_commit_stack_growth(len: usize, capacity: usize, tracker: &ResourceTracker) -> Result<(), ResourceError> {
    if len == capacity {
        tracker.check_allocation(2 * capacity * size_of::<GatherCommit>())
    } else {
        Ok(())
    }
}

/// Polls a gather that may already have settled, returning `None` when it is
/// still `Pending` and so needs committing.
///
/// Shared by the `await` site and the commit walk, which face the same four
/// states: a cached result to replay, a cached error to re-raise, an in-flight
/// gather someone else owns, or work to do.
fn poll_settled_gather<'h>(
    gather: &HeapReadMut<'h, GatherFuture>,
    heap: &mut HeapReader<'h>,
) -> Result<Option<Value>, RunError> {
    match &gather.get(heap).state {
        GatherState::Pending => Ok(None),
        GatherState::Completed(value) => Ok(Some(value.clone_with_heap(heap))),
        GatherState::Failed(_) => {
            let GatherState::Failed(mut error) = mem::replace(&mut gather.get_mut(heap).state, GatherState::Pending)
            else {
                unreachable!("failed gather")
            };
            error.materialize_object(heap);
            let raised = error.clone_with_heap(heap);
            gather.get_mut(heap).state = GatherState::Failed(error);
            Err(raised)
        }
        // TODO: support concurrent re-await (CPython does).
        GatherState::Awaited(_) => Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "cannot reuse gather that is currently being awaited",
        )
        .into()),
    }
}

/// Outcome of [`HeapReadMut::resolve_child`] when a gather has finished driving
/// its children successfully.
///
/// `list_id` is the cached result list to hand back; `waiter` is the
/// downstream that should receive it.
pub(crate) struct GatherSuccess {
    pub list_id: HeapId,
    pub awaiter: Awaiter,
}

impl<'h> HeapReadMut<'h, GatherFuture> {
    /// Caches `list_id` as the gather's successful result.
    ///
    /// Inc_refs `list_id` so the cached state and the caller both own a ref
    /// to the resulting list, then overwrites the state with
    /// `GatherState::Completed(list_id)`. Used directly by
    /// [`VM::await_gather_future`] for the synchronous-completion paths
    /// (empty gather, all externals already resolved); on the async path the
    /// transition happens inside [`Self::resolve_child`].
    pub(crate) fn cache_result(&self, heap: &mut HeapReader<'h>, list_id: HeapId) {
        heap.inc_ref(list_id);
        self.get_mut(heap).state = GatherState::Completed(Value::Ref(list_id));
    }

    /// Records a child's value in its result slots, completing the gather when no children remain.
    /// Returns the completed result and awaiter, or `None` while other children are pending.
    /// A gather that already failed discards later values while its siblings continue running.
    fn resolve_child(&self, vm: &mut VM<'h>, child_id: HeapId, value: Value) -> Option<GatherSuccess> {
        // A sibling failed (or the commit pass rolled back) while this child
        // was still running: it has a result, and nowhere for it to go.
        // `Pending` is not reachable — a child only exists once awaited.
        match &self.get(vm.heap).state {
            GatherState::Awaited(_) => {}
            GatherState::Completed(_) | GatherState::Failed(_) => {
                value.drop_with(vm.heap);
                return None;
            }
            GatherState::Pending => panic!("resolve_child called on a gather that was never awaited"),
        }

        // Remove this child's slot-index mapping.
        let indices: SmallVec<[usize; 1]> = self
            .get_mut(vm.heap)
            .as_awaited_mut()
            .expect("resolve_child called on non-Awaited gather")
            .pending_children
            .remove(&child_id)
            .expect("resolve_child: child not registered with this gather");

        // Take `results` out so the writes can do their clones (which need
        // `&Heap` access) without fighting the `&mut`-chain that
        // `as_awaited_mut` requires. We put it back into the gather right
        // after, before the completion scan.
        let mut results = mem::take(
            &mut self
                .get_mut(vm.heap)
                .as_awaited_mut()
                .expect("resolve_child called on non-Awaited gather")
                .results,
        );
        if let Some((last, init)) = indices.split_last() {
            for &idx in init {
                results[idx] = Some(value.clone_with_heap(vm.heap));
            }
            results[*last] = Some(value);
        } else {
            value.drop_with(vm.heap);
        }

        // Restore results and check completion.
        let awaited = self
            .get_mut(vm.heap)
            .as_awaited_mut()
            .expect("gather still Awaited after recording child resolution");
        awaited.results = results;

        if !awaited.pending_children.is_empty() {
            return None;
        }

        // All children resolved successfully — build the result list.
        // Extract this gather's awaiter (transferred into the returned
        // `GatherSuccess`); the `Awaited` state remains in place until
        // `cache_result` overwrites it, but its `awaiter` field is now the
        // placeholder so dropping the `Awaited` payload won't double-drop
        // the owned `Awaiter`.
        let results = mem::take(&mut awaited.results);
        let awaiter = mem::replace(&mut awaited.awaiter, Awaiter::Task(TaskId::default()));
        let results: Vec<Value> = results
            .into_iter()
            .map(|r| r.expect("all results filled when gather is complete"))
            .collect();
        let list_id = vm.heap.allocate(HeapData::List(List::new(results)));
        self.cache_result(vm.heap, list_id);
        Some(GatherSuccess { list_id, awaiter })
    }

    /// Caches the first failure and transfers the gather's waiter to the caller.
    /// Returns `None` if already settled. Children keep running and retain their references
    /// to this gather, so none can free it while the caller holds this heap reader.
    pub(crate) fn fail(&self, heap: &mut HeapReader<'h>, error: &RunError) -> Option<Awaiter> {
        // Take the Awaited bookkeeping. The state stays `Awaited` (with
        // placeholder fields) until the state replace below commits the
        // transition. The extracted `awaiter` is transferred to the caller
        // — it owns any `GatherSlot` inc_ref it carried. Already settled means
        // a chain that ends here: an earlier failure cached its error and
        // handed the waiter on.
        let (waiter, results) = {
            let awaited = self.get_mut(heap).as_awaited_mut()?;
            (
                mem::replace(&mut awaited.awaiter, Awaiter::Task(TaskId::default())),
                mem::take(&mut awaited.results),
            )
        };

        // Cache a clone so re-awaits replay the same exception.
        self.get_mut(heap).state = GatherState::Failed(error.clone_with_heap(heap));

        // Drop fanned-out result Values that won't reach the waiter. The
        // `pending_children` map goes with the `Awaited` payload; its keys are
        // borrowed ids, owned by `items`.
        results.drop_with(heap);

        Some(waiter)
    }
}

#[cfg(test)]
impl VM<'_> {
    /// Direct ordinary-coroutine admission, without going through host syntax's
    /// intentionally generator-only yield-from lowering.
    pub(crate) fn compatibility11_direct_admission(&mut self, mode: &str) {
        if mode != "ordinary" {
            self.cancel_native("FIRST").unwrap();
            if mode == "delivered" {
                self.cancel_native("REPEAT").unwrap();
            }
            let cancel = self.scheduler.main_task().unwrap().native_cancel.as_ref().unwrap();
            assert_eq!(cancel.delivered, mode == "delivered");
            assert_eq!(cancel.reason, "FIRST");
        }
        let id = self
            .globals
            .iter()
            .find_map(|value| match value {
                Value::Ref(id) if matches!(self.heap.get(*id), HeapData::Coroutine(_)) => Some(*id),
                _ => None,
            })
            .unwrap();
        let HeapReadOutput::Coroutine(coro) = self.heap.read(id) else {
            unreachable!()
        };
        assert_eq!(coro.get(self.heap).state, CoroutineState::New);
        let before = (self.stack.len(), self.suspended_frames.len(), self.recursion_depth);
        let interns = minicbor_serde::to_vec(self.interns).unwrap();
        let namespace = minicbor_serde::to_vec(&coro.get(self.heap).namespace).unwrap();
        let result = self.await_coroutine(&coro);
        if mode == "pending" {
            let Err(error) = result else {
                panic!("legacy admission must refuse")
            };
            assert!(
                matches!(&error, RunError::UncatchableExc(exc) if exc.exc.exc_type() == ExcType::RuntimeError && exc.exc.arg().map(String::as_str) == Some("native cancellation requires catchable cancellation policy 7"))
            );
            error.drop_with(self);
            assert_eq!(coro.get(self.heap).state, CoroutineState::New);
            assert_eq!(
                minicbor_serde::to_vec(&coro.get(self.heap).namespace).unwrap(),
                namespace
            );
            assert_eq!(
                (self.stack.len(), self.suspended_frames.len(), self.recursion_depth),
                before
            );
            // Cached historical frozen admission must precede argument transfer,
            // frame installation and any publication to the intern tables.
            let Err(error) = self.call_frozen_exact(
                crate::frozen::FrozenFunction::FunctoolsReduce,
                [
                    Value::None,
                    Value::None,
                    Value::None,
                    Value::None,
                    Value::None,
                    Value::None,
                ],
            ) else {
                panic!("frozen legacy admission must refuse")
            };
            assert!(matches!(&error, RunError::UncatchableExc(_)));
            error.drop_with(self);
            assert_eq!(
                (self.stack.len(), self.suspended_frames.len(), self.recursion_depth),
                before
            );
            assert_eq!(minicbor_serde::to_vec(self.interns).unwrap(), interns);
        } else {
            assert!(matches!(result.unwrap(), AwaitResult::FramePushed));
            assert_eq!(coro.get(self.heap).state, CoroutineState::Running);
            assert_eq!(self.suspended_frames.len(), before.1 + 1);
        }
    }

    /// Use the scheduler's real save path, without introducing another task.
    pub(crate) fn compatibility11_save_main_task(&mut self) {
        self.save_task_context(TaskId::default());
        self.current_frame.park(self.module_code);
        self.scheduler.set_current_task(None);
    }

    /// A host-only unit-test seam: put the task's handled outer exception in
    /// the unrelated parked VM stack during the real cancellation call. Retire
    /// those temporary owners before normal task activation replaces that stack.
    pub(crate) fn compatibility11_cancel_with_unrelated_parked_stack(&mut self) -> RunResult<()> {
        assert!(self.current_frame.is_parked);
        assert!(self.exception_stack.is_empty());
        let task = self.scheduler.main_task().unwrap();
        assert!(task.frames.len() > 1);
        assert!(task.exception_stack.len() >= 2);
        let count = task.exception_stack.len();
        for _ in 0..count {
            self.exception_stack.push(CaughtException::Value(
                task.exception_stack[0].value().clone_with_heap(self.heap),
            ));
        }
        let parked = self.native_caught_diagnostic(&self.exception_stack[0]).unwrap();
        assert_eq!(parked.message.as_deref(), Some("PARKED"));
        eprintln!(
            "compatibility11 saved frames={:?}, instruction_ip={}, exception_count={count}",
            task.frames
                .iter()
                .map(|f| (f.function_id, f.ip, f.exception_stack_base))
                .collect::<Vec<_>>(),
            task.instruction_ip
        );
        let result = self.cancel_native("FIRST");
        while let Some(value) = self.exception_stack.pop() {
            value.drop_with(self);
        }
        result
    }
}
