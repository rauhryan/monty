//! Admission-scoped barriers before authored cleanup; public Tasks remain run-owned.
use std::mem;

use super::{AwaitResult, VM, scheduler::CleanupLifetime};
use crate::{
    asyncio::{Awaiter, CallId, ExternalFuture, ExternalFutureState, TaskId},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapReadOutput},
    value::Value,
};

impl VM<'_> {
    /// A first-error gather may leave live continuations while its caller recovers.
    /// Recovery remains ordinary code. Entering that frame's cleanup closes these
    /// directly admitted obligations: cancel, then drain, before its first instruction.
    /// Transferred descendants retain their established normal-wait contract.
    /// Recovery before entry can satisfy dependencies on ordinary parent body code.
    pub(super) fn ordinary_cleanup_barrier(&mut self) -> Option<RunResult<AwaitResult>> {
        let owner = self.scheduler.current_task_id()?;
        let depth = self.suspended_frames.len();
        let task = self.scheduler.get_task_mut(owner);
        if task.cleanup_shutdown_frame == Some(depth) {
            let id = task.cleanup_shutdown.take().expect("cleanup barrier");
            task.cleanup_shutdown_frame = None;
            self.pop().drop_with(self);
            Value::Ref(id).drop_with(self);
            return None;
        }
        if !self
            .current_frame
            .code
            .native_cleanup_entries
            .as_ref()
            .is_some_and(|entries| entries.contains(&u32::try_from(self.instruction_ip).expect("instruction offset")))
        {
            return None;
        }
        let children: Vec<_> = self
            .scheduler
            .get_task_mut(owner)
            .gather_children
            .clone()
            .into_iter()
            .filter(|child| {
                self.scheduler
                    .task(*child)
                    .is_some_and(|task| task.cleanup_parent_frame == Some(depth))
            })
            .collect();
        let calls: Vec<_> = self
            .scheduler
            .task(owner)
            .expect("owner")
            .cleanup_externals
            .iter()
            .filter_map(|(call, frame)| {
                (*frame == depth && self.scheduler.gather_externals.get(call) == Some(&owner)).then_some(*call)
            })
            .collect();
        if children.is_empty() && calls.is_empty() {
            return None;
        }
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.state = ExternalFutureState::Pending {
            awaiter: Some(Awaiter::Task(owner)),
        };
        let id = self.heap.allocate(HeapData::ExternalFuture(Box::new(future)));
        let task = self.scheduler.get_task_mut(owner);
        task.cleanup_shutdown = Some(id);
        task.cleanup_shutdown_frame = Some(depth);
        task.native_protected = true;
        self.scheduler.block_current_on(id, self.heap);
        for child in children {
            if self.scheduler.task(child).expect("live child").cleanup_transferred == CleanupLifetime::Admitted {
                self.cancel_public_task_reason(child, "cleanup-exit");
            }
        }
        for call in calls {
            self.fail_future(call.raw(), ExcType::value_error("cleanup-exit"));
        }
        self.finish_ordinary_cleanup(owner);
        Some(self.switch_or_yield())
    }

    /// Move unfinished obligations with their synchronous/awaited caller when a
    /// helper returns. A deeper helper's cleanup does not own its caller's work.
    pub(super) fn leave_cleanup_frame(&mut self) {
        let Some(owner) = self.scheduler.current_task_id() else {
            return;
        };
        let depth = self.suspended_frames.len();
        let children = self.scheduler.get_task_mut(owner).gather_children.clone();
        for child in children {
            let task = self.scheduler.get_task_mut(child);
            if task.cleanup_parent_frame == Some(depth) {
                task.cleanup_parent_frame = depth.checked_sub(1);
            }
        }
        for (_, frame) in &mut self.scheduler.get_task_mut(owner).cleanup_externals {
            if *frame == depth {
                *frame = depth.saturating_sub(1);
            }
        }
    }

    pub(super) fn finish_ordinary_cleanup(&mut self, owner: TaskId) {
        let Some(task) = self.scheduler.task(owner) else { return };
        let Some(id) = task.cleanup_shutdown else { return };
        let depth = task.cleanup_shutdown_frame;
        if task.gather_children.iter().any(|child| {
            self.scheduler
                .task(*child)
                .is_some_and(|task| task.cleanup_parent_frame == depth)
        }) || task
            .cleanup_externals
            .iter()
            .any(|(call, frame)| Some(*frame) == depth && self.scheduler.gather_externals.get(call) == Some(&owner))
        {
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
            self.deliver_awaiter_success(awaiter, Value::None);
        }
    }
}
