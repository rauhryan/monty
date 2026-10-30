//! Lexical guest cancellation over the existing scheduler, separate from host latches.
use super::VM;
use crate::{
    args::ArgValues,
    asyncio::{
        Awaiter, CallId, CancelScope, CancelScopeState, ExternalFuture, ExternalFutureState, TaskGroupState, TaskId,
    },
    bytecode::CallResult,
    exception_private::{ExcType, ExcTypeExt, ExceptionRaise, RunError, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    modules::{
        ModuleFunctions,
        anyio::{self, AnyioFunctions},
    },
    types::BoundMethod,
    value::{EitherStr, Value},
};

impl VM<'_> {
    pub(crate) fn is_anyio_scope(&self, id: HeapId) -> bool {
        matches!(self.heap.get(id), HeapData::ExternalFuture(f) if f.cancel_scope.is_some())
    }

    pub(crate) fn new_anyio_scope(&mut self, shield: bool) -> RunResult<Value> {
        let task = self.scheduler.current_task().expect("scope constructor task");
        if task.anyio_run == 0 || (task.run_frame.is_none() && task.run_owner.is_none()) {
            return Err(scope_error("CancelScope requires an active asyncio.run"));
        }
        let (owner, run) = (task.id, task.anyio_run);
        let id = self.scheduler.allocate_anyio_id();
        let mut future = ExternalFuture::new_pending(CallId::new(u32::MAX), None);
        future.cancel_scope = Some(CancelScope {
            id,
            owner,
            run,
            state: CancelScopeState::Created,
            shield,
            cancel_called: false,
            cancelled_caught: false,
            signal: None,
        });
        Ok(Value::Ref(
            self.heap.allocate(HeapData::ExternalFuture(Box::new(future))),
        ))
    }

    fn scope(&self, id: HeapId) -> &CancelScope {
        let HeapData::ExternalFuture(f) = self.heap.get(id) else {
            unreachable!()
        };
        f.cancel_scope.as_ref().expect("scope carrier")
    }

    pub(crate) fn anyio_scope_attr(&mut self, id: HeapId, attr: &EitherStr) -> Option<CallResult> {
        let scope = self.scope(id);
        let value = match attr.as_str(self.interns) {
            "cancel_called" => Value::Bool(scope.cancel_called),
            "cancelled_caught" => Value::Bool(scope.cancelled_caught),
            "shield" => Value::Bool(scope.shield),
            name @ ("cancel" | "__enter__" | "__exit__") => {
                let function = match name {
                    "__enter__" => AnyioFunctions::ScopeEnter,
                    "__exit__" => AnyioFunctions::ScopeExit,
                    _ => AnyioFunctions::ScopeCancel,
                };
                self.heap.inc_ref(id);
                Value::Ref(self.heap.allocate(HeapData::BoundMethod(BoundMethod {
                    instance: Value::Ref(id),
                    func: Value::ModuleFunction(ModuleFunctions::Anyio(function)),
                })))
            }
            _ => return None,
        };
        Some(CallResult::Value(value))
    }

    pub(crate) fn call_anyio_scope(&mut self, id: HeapId, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let function = match attr.as_str(self.interns) {
            "cancel" => Some(AnyioFunctions::ScopeCancel),
            "__enter__" => Some(AnyioFunctions::ScopeEnter),
            "__exit__" => Some(AnyioFunctions::ScopeExit),
            _ => None,
        };
        let Some(function) = function else {
            args.drop_with(self);
            return Err(ExcType::attribute_error("CancelScope", attr.as_str(self.interns)));
        };
        self.heap.inc_ref(id);
        anyio::call(self, function, args.prepend(Value::Ref(id)))
    }

    pub(crate) fn anyio_scope_context(
        &mut self,
        id: HeapId,
        exc: Option<HeapId>,
        enter: bool,
    ) -> RunResult<CallResult> {
        let owner = self.scheduler.current_task_id().expect("scope task");
        if enter {
            let scope = self.scope(id);
            if scope.owner != owner || scope.run != self.scheduler.task(owner).expect("owner").anyio_run {
                return Err(scope_error("CancelScope entry is foreign"));
            }
            if scope.state != CancelScopeState::Created {
                return Err(scope_error(
                    "Each CancelScope may only be used for a single 'with' block",
                ));
            }
            let HeapReadOutput::ExternalFuture(f) = self.heap.read(id) else {
                unreachable!()
            };
            f.get_mut(self.heap).cancel_scope.as_mut().unwrap().state = CancelScopeState::Active;
            self.heap.inc_ref(id);
            self.scheduler.get_task_mut(owner).anyio_scopes.push(id);
            self.heap.inc_ref(id);
            Ok(CallResult::Value(Value::Ref(id)))
        } else {
            let suppressed = self.exit_anyio_scope(owner, id, exc)?;
            Ok(CallResult::Value(Value::Bool(suppressed)))
        }
    }

    pub(super) fn exit_anyio_scope(&mut self, owner: TaskId, id: HeapId, exc: Option<HeapId>) -> RunResult<bool> {
        let scope = self.scope(id);
        if scope.owner != owner || scope.run != self.scheduler.task(owner).expect("owner").anyio_run {
            return Err(scope_error("CancelScope exit is foreign"));
        }
        if scope.state != CancelScopeState::Active {
            return Err(scope_error("CancelScope is not active"));
        }
        if self.scheduler.task(owner).expect("owner").anyio_scopes.last() != Some(&id) {
            return Err(scope_error("CancelScope must exit in LIFO order"));
        }
        let suppressed = exc.is_some_and(|exc| matches!(scope.signal, Some(Value::Ref(signal)) if signal == exc));
        let HeapReadOutput::ExternalFuture(f) = self.heap.read(id) else {
            unreachable!()
        };
        let scope = f.get_mut(self.heap).cancel_scope.as_mut().unwrap();
        scope.state = CancelScopeState::Exited;
        scope.cancelled_caught = suppressed;
        self.scheduler.get_task_mut(owner).anyio_scopes.pop();
        Value::Ref(id).drop_with(self);
        self.refresh_anyio_cancellation();
        Ok(suppressed)
    }

    /// Innermost cancelled origin wins; a shield masks ancestors, never itself.
    fn effective_anyio_scope(&self, task: TaskId) -> Option<HeapId> {
        for id in self.scheduler.task(task)?.anyio_scopes.iter().rev() {
            let scope = self.scope(*id);
            if scope.state == CancelScopeState::Active {
                if scope.cancel_called {
                    return Some(*id);
                }
                if scope.shield {
                    return None;
                }
            }
        }
        None
    }

    pub(super) fn anyio_result_cancelled(&self, result: &RunResult<Value>) -> bool {
        let Err(RunError::Exc(error)) = result else {
            return false;
        };
        let Some(Value::Ref(signal)) = &error.object else {
            return false;
        };
        self.scheduler.current_task().is_some_and(|task| {
            task.anyio_scopes
                .iter()
                .any(|id| matches!(self.scope(*id).signal, Some(Value::Ref(id)) if id == *signal))
        })
    }

    pub(super) fn anyio_scope_signal_id(&self, scope: HeapId) -> Option<HeapId> {
        if let Some(Value::Ref(id)) = self.scope(scope).signal {
            Some(id)
        } else {
            None
        }
    }

    pub(super) fn anyio_scope_owns_exception(&self, scope: HeapId, exc: Option<HeapId>) -> bool {
        exc.is_some() && exc == self.anyio_scope_signal_id(scope)
    }

    pub(super) fn anyio_exception_cancelled(&self, exc: HeapId) -> bool {
        self.scheduler.current_task().is_some_and(|t| {
            t.anyio_scopes
                .iter()
                .any(|scope| self.anyio_scope_signal_id(*scope) == Some(exc))
        })
    }

    fn scope_signal(&mut self, id: HeapId) -> RunError {
        let Some(Value::Ref(signal)) = &self.scope(id).signal else {
            unreachable!("cancelled scope signal")
        };
        let signal = *signal;
        let HeapData::Exception(object) = self.heap.get(signal) else {
            unreachable!()
        };
        let mut error: ExceptionRaise = object.simple.clone().into();
        self.heap.inc_ref(signal);
        error.object = Some(Value::Ref(signal));
        RunError::Exc(error)
    }

    /// Barriers drain admitted children even when the owning guest scope is cancelled.
    fn anyio_exit_barrier(&self, task: TaskId) -> bool {
        self.scheduler.blocked_on(task).is_some_and(|id| {
            matches!(self.heap.get(id),
            HeapData::ExternalFuture(f) if f.task_group.as_ref().is_some_and(|g| g.state != TaskGroupState::Active))
        })
    }

    pub(super) fn anyio_checkpoint(&mut self) -> Option<RunError> {
        let task = self.scheduler.current_task_id()?;
        // Host delivery arbitration remains in the existing native paths. Scope
        // delivery never clears or mutates that latch, even if its signal is caught.
        if self.anyio_exit_barrier(task) {
            return None;
        }
        let id = self.effective_anyio_scope(task)?;
        Some(self.scope_signal(id))
    }

    pub(crate) fn cancel_anyio_scope(&mut self, id: HeapId) -> RunResult<()> {
        let task = self.scheduler.current_task().expect("scope cancel task");
        // Cancellation may be requested by a sibling in the same run; lexical
        // enter/exit ownership never transfers. A stale run cannot mutate it.
        if self.scope(id).run != task.anyio_run {
            return Err(scope_error("CancelScope cancellation is foreign to this run"));
        }
        if self.scope(id).cancel_called {
            return Ok(());
        }
        let mut signal = ExcType::value_error("cancelled by AnyIO CancelScope");
        signal.materialize_object(self.heap);
        let RunError::Exc(mut signal) = signal else {
            unreachable!()
        };
        let HeapReadOutput::ExternalFuture(f) = self.heap.read(id) else {
            unreachable!()
        };
        let scope = f.get_mut(self.heap).cancel_scope.as_mut().unwrap();
        scope.cancel_called = true;
        scope.signal = signal.object.take();
        drop(f);
        self.refresh_anyio_cancellation();
        Ok(())
    }

    /// An inherited shield ending can expose cancellation on a different task's
    /// already-blocked checkpoint. Only that task's route is woken; exiting the
    /// owner itself still observes the ancestor at its next checkpoint.
    fn refresh_anyio_cancellation(&mut self) {
        for task in self.scheduler.anyio_task_ids() {
            if self.effective_anyio_scope(task).is_some() && !self.anyio_exit_barrier(task) {
                self.wake_anyio_cancel(task);
            }
        }
    }

    /// Detach just this wait route; queue revocation and late host settlements
    /// use the same scheduler primitives as native cancellation, without a latch.
    fn wake_anyio_cancel(&mut self, task: TaskId) {
        let Some(wait) = self.scheduler.blocked_on(task) else {
            return;
        };
        let Some(scope) = self.effective_anyio_scope(task) else {
            return;
        };
        // Direct external gather slots have no Task to signal. Revoke only
        // this interrupted await graph, never unrelated earlier admissions.
        let mut pending = vec![wait];
        let mut calls = Vec::new();
        while let Some(item) = pending.pop() {
            match self.heap.get(item) {
                HeapData::GatherFuture(g) => pending.extend(g.items.iter().copied()),
                HeapData::ExternalFuture(f) if self.scheduler.gather_externals.get(&f.call_id) == Some(&task) => {
                    calls.push(f.call_id);
                }
                _ => {}
            }
        }
        for call in calls {
            self.scheduler.gather_externals.remove(&call);
            let signal = self.scope_signal(scope);
            self.fail_future(call.raw(), signal);
        }
        // The final slot may already have woken the task with its own signal.
        if self.scheduler.blocked_on(task).is_none() {
            return;
        }
        self.cancel_queue_operation(task);
        let signal = self.scope_signal(scope);
        let HeapReadOutput::ExternalFuture(f) = self.heap.read(wait) else {
            // Gather routes own their downstream. Fail the route, preserving
            // inherited descendant lifetime roots for the normal drain policy.
            let HeapReadOutput::GatherFuture(g) = self.heap.read(wait) else {
                unreachable!()
            };
            let waiter = g.fail(self.heap, &signal);
            drop(g);
            waiter.drop_with(self.heap);
            self.deliver_awaiter_failure(Awaiter::Task(task), signal);
            return;
        };
        let external = f.get(self.heap).child.is_none()
            && f.get(self.heap).queue_op.is_none()
            && f.get(self.heap).public_task.is_none()
            && f.get(self.heap).task_group.is_none();
        let call = f.get(self.heap).call_id.raw();
        if external {
            drop(f);
            self.fail_future(call, signal);
        } else {
            if let Some(p) = &mut f.get_mut(self.heap).public_task {
                p.waiters.retain(|a| !matches!(a, Awaiter::Task(id) if *id == task));
            } else if let ExternalFutureState::Pending { awaiter } = &mut f.get_mut(self.heap).state {
                awaiter.take().drop_with(self.heap);
            }
            drop(f);
            self.scheduler
                .get_task_mut(task)
                .pending_join
                .take()
                .map(Value::Ref)
                .drop_with(self.heap);
            self.deliver_awaiter_failure(Awaiter::Task(task), signal);
        }
    }
}

fn scope_error(message: &str) -> RunError {
    SimpleException::new_msg(ExcType::RuntimeError, message).into()
}
