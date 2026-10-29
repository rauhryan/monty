//! Task scheduler for async execution and call ID allocation.
//!
//! # Task Model
//!
//! The current task's context lives in the VM; all other contexts live in their tasks.
//! Task 0 is the main task. Spawned tasks deliver to a gather or an owned child handle.

use std::{collections::VecDeque, mem};

use ahash::AHashMap;
use smallvec::{SmallVec, smallvec};

use super::{FrameNamespace, exceptions::CaughtException, guest_queue::finish_queue_operation};
use crate::{
    asyncio::{Awaiter, CallId, TaskId},
    exception_private::{RunError, RunResult, SimpleException},
    heap::{ContainsHeap, DropWithContext, Heap, HeapId, HeapReadOutput, HeapReader},
    intern::{FunctionId, StringId},
    value::Value,
};

/// Live tasks are runnable or blocked; completion removes the task from the scheduler.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum TaskState {
    /// Queued or currently executing. `Err` must be raised before executing more bytecode.
    Ready(RunResult<()>),
    /// Owns an inc_ref on the awaitable; its awaiter identifies the task to wake.
    Blocked(HeapId),
}

impl<C: ContainsHeap> DropWithContext<C> for TaskState {
    fn drop_with(self, heap: &mut C) {
        match self {
            Self::Ready(Err(error)) => error.drop_with(heap),
            Self::Ready(Ok(())) => {}
            Self::Blocked(id) => heap.heap_mut().dec_ref(id),
        }
    }
}

/// A live async task. Its execution context is saved here while another task runs.
/// The current task's frames and stacks live in the VM, including for the main task.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Task {
    /// Unique identifier for this task.
    pub id: TaskId,
    /// Saved frames; empty while this task is current or has not started.
    pub frames: Vec<SerializedTaskFrame>,
    /// Saved operand stack; the current task uses the VM's stack.
    pub stack: Vec<Value>,
    /// Exception stack for nested except blocks.
    pub exception_stack: Vec<CaughtException>,
    /// VM-level instruction_ip (for exception table lookup).
    pub instruction_ip: usize,
    /// Owned coroutine reference for a spawned task; the main task has none.
    pub coroutine_id: Option<HeapId>,
    /// Result recipient, owning an inc_ref for a gather. An already-settled gather
    /// discards the result. The main task has no awaiter.
    pub awaiter: Option<Awaiter>,
    /// Owned child outcome and scope roots. Removed on delivery or drain.
    #[serde(default)]
    pub child_result: Option<HeapId>,
    /// Owned identity of the one injected child cancellation exception.
    #[serde(default)]
    pub child_cancel_signal: Option<Value>,
    /// Owned roots for unfinished children and undelivered ordinary failures.
    #[serde(default)]
    pub children: Vec<HeapId>,
    /// Lifetime edges are independent of gather's first-result routing. The
    /// scheduler owns these tasks; IDs here do not own heap references.
    #[serde(default)]
    pub gather_children: Vec<TaskId>,
    #[serde(default)]
    pub lifetime_parent: Option<TaskId>,
    /// A gather invoked by protected cleanup protects its spawned continuations.
    #[serde(default)]
    pub gather_protected: bool,
    /// Ordinary gather failures which no waiter received, owned until scope drain.
    #[serde(default)]
    pub gather_errors: Vec<Value>,
    /// The queued error came through gather and is not consumed until activation.
    #[serde(default)]
    pub pending_gather_error: GatherErrorDelivery,
    /// Owned opaque identity; handles keep their own clone after scope exit.
    #[serde(default)]
    pub scope_token: Option<Value>,
    #[serde(default)]
    pub scope_open: bool,
    /// Owned join root until this task actually resumes.
    #[serde(default)]
    pub pending_join: Option<HeapId>,
    /// Owned queue operation until activation commits or cancels its reservation.
    #[serde(default)]
    pub pending_queue: Option<HeapId>,
    /// Owned drain future while scope exit waits for children.
    #[serde(default)]
    pub drain: Option<HeapId>,
    /// Owning roots for nested public groups; membership borrows the owner's root.
    #[serde(default)]
    pub groups: Vec<HeapId>,
    #[serde(default)]
    pub group_parent: Option<HeapId>,
    /// The interrupted body waits only for its current await graph, rather
    /// than unrelated children of an outer group owned by the same task.
    #[serde(default)]
    pub group_body_waits: Option<Vec<TaskId>>,
    /// Root normal-return shutdown barrier; separate from physical disposal.
    #[serde(default)]
    pub root_shutdown: Option<HeapId>,
    #[serde(default)]
    pub root_shutdown_error: Option<RunError>,
    /// Exact frame installed by the public run call, independent of sync wrappers.
    /// Older producers did not record a run boundary.
    #[serde(default)]
    pub run_frame: Option<usize>,
    /// Enclosing explicit run inherited at task admission; a borrowed live task ID.
    #[serde(default)]
    pub run_owner: Option<TaskId>,
    /// Frame in the lifetime parent which admitted this gather continuation.
    /// Public Tasks have no implicit lexical cleanup obligation.
    #[serde(default)]
    pub cleanup_parent_frame: Option<usize>,
    /// Transferred descendants retain the established normal-wait contract.
    #[serde(default)]
    pub cleanup_transferred: CleanupLifetime,
    /// Borrowed direct external gather slots and their admitting frames.
    #[serde(default)]
    pub cleanup_externals: Vec<(CallId, usize)>,
    /// Protected wait before an admitting frame enters authored cleanup.
    #[serde(default)]
    pub cleanup_shutdown: Option<HeapId>,
    #[serde(default)]
    pub cleanup_shutdown_frame: Option<usize>,
    /// Current execution state.
    state: TaskState,
    /// First cancellation cause, independent of guest exceptions and physical disposal.
    #[serde(default)]
    pub native_cancel: Option<NativeCancellation>,
    /// Lexical cleanup owns the suspended wait; persisted, never guessed from names.
    #[serde(default)]
    pub native_protected: bool,
}

/// Distinct cleanup policies are lifetime roles, not independent task flags.
/// Keep the bool representation written by initial round8 producers.
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "bool", into = "bool")]
pub(crate) enum CleanupLifetime {
    #[default]
    Admitted,
    Transferred,
}

impl From<bool> for CleanupLifetime {
    fn from(transferred: bool) -> Self {
        if transferred { Self::Transferred } else { Self::Admitted }
    }
}

impl From<CleanupLifetime> for bool {
    fn from(policy: CleanupLifetime) -> Self {
        policy == CleanupLifetime::Transferred
    }
}

/// A queued gather error remains unconsumed until task activation.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GatherErrorDelivery {
    #[default]
    None,
    Pending,
    /// The ready error owns its roots but cannot run until protected children drain.
    Draining,
}

/// Plain owned metadata: no heap edges or guest-controlled latch.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct NativeCancellation {
    /// Immutable first host cause; an empty string is still present.
    pub reason: String,
    /// The single catchable delivery has consumed the body wait.
    pub delivered: bool,
    /// Stale settlements for this detached external operation are ignored.
    pub revoked: Option<u32>,
    /// Distinguishes legacy delivered+null from current delivered+null.
    #[serde(default)]
    pub policy: u8,
    /// Frozen once at delivery. Plain metadata never roots a guest object.
    #[serde(default)]
    pub interrupted_error: Option<InterruptedError>,
}

/// Bounded borrowed-metadata projection, independent of the exception args graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct InterruptedError {
    pub exc_type: monty_types::ExcType,
    pub message: Option<String>,
    pub message_truncated: bool,
}

impl InterruptedError {
    pub(crate) fn capture(exc: &SimpleException) -> Self {
        let message = exc.arg().map(String::as_str);
        let message_truncated = message.is_some_and(|s| s.len() > 4096);
        let message = message.map(|s| {
            let mut end = s.len().min(4096);
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s[..end].to_owned()
        });
        Self {
            exc_type: exc.exc_type(),
            message,
            message_truncated,
        }
    }
}

impl<C: ContainsHeap> DropWithContext<C> for Task {
    fn drop_with(mut self, heap: &mut C) {
        self.stack.drain(..).drop_with(heap);
        self.exception_stack.drain(..).drop_with(heap);
        for frame in self.frames.drain(..) {
            frame.namespace.drop_with(heap);
            frame.generator_activation.drop_with(heap);
        }
        for id in self.groups.drain(..) {
            heap.heap_mut().dec_ref(id);
        }
        self.scope_token.take().drop_with(heap);
        self.child_cancel_signal.take().drop_with(heap);
        self.gather_errors.drain(..).drop_with(heap);
        self.state.drop_with(heap);
        self.root_shutdown_error.take().drop_with(heap);
        for id in self
            .children
            .drain(..)
            .chain(self.child_result.take())
            .chain(self.pending_join.take())
            .chain(self.pending_queue.take())
            .chain(self.drain.take())
            .chain(self.root_shutdown.take())
            .chain(self.cleanup_shutdown.take())
        {
            heap.heap_mut().dec_ref(id);
        }
        if let Some(coro_id) = self.coroutine_id.take() {
            heap.heap_mut().dec_ref(coro_id);
        }
        if let Some(awaiter) = self.awaiter.take() {
            awaiter.drop_with(heap);
        }
    }
}

/// Serialized call frame for task storage.
///
/// Similar to `SerializedFrame` but used within the scheduler for task context.
/// Cannot store `&Code` references - uses `FunctionId` to look up code on resume.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct SerializedTaskFrame {
    /// Which function's code this frame executes (None = module-level).
    pub function_id: Option<FunctionId>,
    /// Instruction pointer within this frame's bytecode.
    pub ip: usize,
    /// Base index into the VM stack for this frame's locals region.
    pub stack_base: usize,
    /// Number of local variable slots (0 for module-level frames).
    pub locals_count: u16,
    /// Base index into the VM-wide `exception_stack` for this frame.
    /// See `CallFrame.exception_stack_base`.
    pub exception_stack_base: usize,
    /// Caller's bytecode offset at the call site (for tracebacks). See
    /// `CallFrame.call_offset`.
    pub call_offset: Option<u32>,
    /// Whether this frame is a class `__init__` (see `CallFrame.is_initializer`).
    pub is_initializer: bool,
    /// Frame namespace, owning its dict references (see `CallFrame.namespace`).
    pub namespace: Option<Box<FrameNamespace>>,
    /// Persisted synchronous generator activation, when this frame drives one.
    #[serde(default)]
    pub generator_activation: Option<super::GeneratorActivation>,
    /// The `from ... import` in progress, if the frame parked inside one.
    #[serde(default)]
    pub import_from_module: Option<StringId>,
}

impl Task {
    /// Creates a runnable task, taking ownership of its coroutine and awaiter references.
    pub fn new(id: TaskId, coroutine_id: Option<HeapId>, awaiter: Option<Awaiter>) -> Self {
        Self {
            id,
            frames: Vec::new(),
            stack: Vec::new(),
            exception_stack: Vec::new(),
            instruction_ip: 0,
            coroutine_id,
            awaiter,
            state: TaskState::Ready(Ok(())),
            child_result: None,
            child_cancel_signal: None,
            children: Vec::new(),
            gather_children: Vec::new(),
            lifetime_parent: None,
            gather_protected: false,
            gather_errors: Vec::new(),
            pending_gather_error: GatherErrorDelivery::None,
            scope_token: None,
            scope_open: false,
            pending_join: None,
            pending_queue: None,
            drain: None,
            groups: Vec::new(),
            group_parent: None,
            group_body_waits: None,
            root_shutdown: None,
            root_shutdown_error: None,
            run_frame: None,
            run_owner: None,
            cleanup_parent_frame: None,
            cleanup_transferred: CleanupLifetime::Admitted,
            cleanup_externals: Vec::new(),
            cleanup_shutdown: None,
            cleanup_shutdown_frame: None,
            native_cancel: None,
            native_protected: false,
        }
    }
}

/// Owns live tasks, pending external futures, and call IDs for both sync and async execution.
/// A blocked task is queued once when its awaitable settles, for either a value or an error.
/// Selecting that task consumes its queue entry and resume result before bytecode can run.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Scheduler {
    /// All tasks keyed by their `TaskId`.
    tasks: AHashMap<TaskId, Task>,
    /// Tasks ready to execute or raise a pending exception.
    ready_queue: VecDeque<TaskId>,
    /// Task whose context is in the VM, or `None` when that context has been discarded.
    current_task: Option<TaskId>,
    /// Counter for generating new task IDs.
    next_task_id: u32,
    /// Counter for external call IDs (always incremented, even for sync resolution).
    next_call_id: u32,
    /// New runs issue only host IDs from this counter. Legacy runs retain strict
    /// pending-ID validation because their counter also allocated internal handles.
    #[serde(default)]
    host_id_floor: Option<u32>,
    /// Host-side index mapping each unresolved external call to its
    /// `HeapData::ExternalFuture` entry. The scheduler holds an inc_ref on
    /// each value until the host resolves or fails the call — that ref keeps
    /// the future entry alive between yield and resume even if no awaiter is
    /// holding a `Value::Ref` to it.
    pending_externals: AHashMap<CallId, HeapId>,
    /// Direct external gather items borrow the live registration's root. Entries
    /// disappear on settlement and move to the enclosing owner on task exit.
    #[serde(default)]
    pub gather_externals: AHashMap<CallId, TaskId>,
    /// Index mapping a spawned coroutine's `HeapId` to the `TaskId` driving
    /// it. Populated in [`Scheduler::spawn`] and removed in
    /// [`Scheduler::cancel_task`]. Lets `GatherFuture` and other call sites
    /// dispatch from coroutine heap id back to the driving task without
    /// scanning all tasks.
    coroutine_to_task: AHashMap<HeapId, TaskId>,
    /// Borrowed live task IDs awaited by protected gathers or forwarded cancellation. Public Tasks may
    /// have several waiters, so these barriers are independent of ownership.
    #[serde(default)]
    pub public_gather_waits: AHashMap<TaskId, Vec<TaskId>>,
}

impl Scheduler {
    /// Creates the main task as current; it needs no queue entry to start executing.
    pub fn new() -> Self {
        let main_task_id = TaskId::default();
        let main_task = Task::new(main_task_id, None, None);
        let mut tasks = AHashMap::new();
        tasks.insert(main_task_id, main_task);
        Self {
            tasks,
            ready_queue: VecDeque::new(), // Main task is current, not in ready queue
            current_task: Some(main_task_id),
            next_task_id: 1,
            next_call_id: 0,
            host_id_floor: Some(0),
            pending_externals: AHashMap::new(),
            gather_externals: AHashMap::new(),
            coroutine_to_task: AHashMap::new(),
            public_gather_waits: AHashMap::new(),
        }
    }

    /// Identifies the VM's loaded context, even when that task is blocked on a host call.
    #[inline]
    pub fn current_task_id(&self) -> Option<TaskId> {
        self.current_task
    }

    /// Whether awaiting the current call can proceed without delaying other work.
    pub fn can_await_eagerly(&self) -> bool {
        self.ready_queue.is_empty() && self.pending_externals.is_empty()
    }

    /// Borrow a task without transferring any of its owned roots.
    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Deterministic breadth-first ownership walk, including a main task parked on a join.
    pub fn owned_task_tree(&self, heap: &HeapReader<'_>) -> Vec<TaskId> {
        self.owned_subtree(TaskId::default(), heap)
    }

    pub fn owned_subtree(&self, root: TaskId, heap: &HeapReader<'_>) -> Vec<TaskId> {
        let mut ids = vec![root];
        let mut index = 0;
        while index < ids.len() {
            if let Some(task) = self.tasks.get(&ids[index]) {
                for child in &task.gather_children {
                    if !ids.contains(child) {
                        ids.push(*child);
                    }
                }
                for child in &task.children {
                    if let HeapReadOutput::ExternalFuture(future) = heap.read(*child)
                        && let Some(id) = future.get(heap).child.as_ref().and_then(|child| child.task)
                        && !ids.contains(&id)
                    {
                        ids.push(id);
                    }
                }
            }
            index += 1;
        }
        ids
    }

    /// Transfer unfinished gather obligations before removing a completed task.
    /// This does not delay its explicit result or cancel any sibling.
    pub fn transfer_gather_lifetime(&mut self, id: TaskId) -> Option<TaskId> {
        let task = self.tasks.get_mut(&id).expect("live task");
        let parent = task.lifetime_parent?;
        let cleanup_frame = task.cleanup_parent_frame;
        let externals = mem::take(&mut task.cleanup_externals);
        let children = mem::take(&mut task.gather_children);
        let errors = mem::take(&mut task.gather_errors);
        for child in &children {
            self.get_task_mut(*child).lifetime_parent = Some(parent);
            self.get_task_mut(*child).cleanup_parent_frame = cleanup_frame;
            self.get_task_mut(*child).cleanup_transferred = CleanupLifetime::Transferred;
        }
        for owner in self.gather_externals.values_mut() {
            if *owner == id {
                *owner = parent;
            }
        }
        if let Some(waiting) = self.public_gather_waits.remove(&id) {
            self.public_gather_waits.entry(parent).or_default().extend(waiting);
        }
        let owner = self.get_task_mut(parent);
        let depth = cleanup_frame.or(owner.run_frame).unwrap_or(0);
        owner
            .cleanup_externals
            .extend(externals.into_iter().map(|(call, _)| (call, depth)));
        owner.gather_children.retain(|child| *child != id);
        owner.gather_children.extend(children);
        owner.gather_errors.extend(errors);
        Some(parent)
    }

    /// Older genuine snapshots may predate ordinary/public root edges. Adopt
    /// otherwise unowned live continuations before logical shutdown, never drop them.
    pub fn adopt_root_tasks(&mut self) {
        let root = TaskId::default();
        let mut children: Vec<_> = self
            .tasks
            .values()
            .filter_map(|task| {
                (!task.id.is_main() && (task.lifetime_parent.is_none() || task.lifetime_parent == Some(root)))
                    .then_some(task.id)
            })
            .collect();
        children.sort_unstable();
        for child in children {
            self.get_task_mut(child).lifetime_parent = Some(root);
            let task = self.get_task_mut(root);
            if !task.gather_children.contains(&child) {
                task.gather_children.push(child);
            }
        }
    }

    /// Membership is captured at admission, not inferred from the execution root.
    pub fn run_tasks(&self, owner: TaskId) -> Vec<TaskId> {
        let mut tasks: Vec<_> = self
            .tasks
            .values()
            .filter_map(|task| (task.run_owner == Some(owner)).then_some(task.id))
            .collect();
        tasks.sort_unstable();
        tasks
    }

    pub fn run_externals(&self, owner: TaskId) -> Vec<CallId> {
        let task = self.task(owner).expect("run owner");
        let Some(depth) = task.run_frame else { return Vec::new() };
        task.cleanup_externals
            .iter()
            .filter_map(|(call, frame)| {
                (*frame >= depth && self.gather_externals.get(call) == Some(&owner)).then_some(*call)
            })
            .collect()
    }

    pub fn group_has_tasks(&self, group: HeapId) -> bool {
        self.tasks.values().any(|task| task.group_parent == Some(group))
    }

    /// Currently loaded task, for task-local cancellation policy.
    pub fn current_task(&self) -> Option<&Task> {
        self.current_task.and_then(|id| self.tasks.get(&id))
    }

    /// Borrow the awaitable root of a suspended child.
    pub fn blocked_on(&self, id: TaskId) -> Option<HeapId> {
        match self.tasks.get(&id)?.state {
            TaskState::Blocked(id) => Some(id),
            TaskState::Ready(_) => None,
        }
    }

    /// Returns the main task, whose context is saved here whenever another
    /// task is loaded; `None` once it has been cancelled.
    pub fn main_task(&self) -> Option<&Task> {
        self.tasks.get(&TaskId::default())
    }

    /// Returns a mutable reference to a task by ID.
    ///
    /// # Panics
    /// Panics if the task ID doesn't exist.
    #[inline]
    pub fn get_task_mut(&mut self, task_id: TaskId) -> &mut Task {
        self.tasks
            .get_mut(&task_id)
            .expect("Scheduler::get_task_mut: task not found")
    }

    /// Allocates a new CallId for an external function call.
    ///
    /// The counter always increments, even for sync resolution, to keep IDs unique.
    pub fn allocate_call_id(&mut self) -> CallId {
        let id = CallId::new(self.next_call_id);
        self.next_call_id += 1;
        id
    }

    /// Constant-space history: every issued host ID below the high-water mark is known.
    pub fn knows_host_call(&self, id: u32) -> bool {
        self.pending_externals.contains_key(&CallId::new(id))
            || self
                .host_id_floor
                .is_some_and(|floor| id >= floor && id < self.next_call_id)
            || self
                .tasks
                .values()
                .any(|task| task.native_cancel.as_ref().and_then(|c| c.revoked) == Some(id))
    }

    /// An absent known registration is settled or revoked; never import its reply again.
    pub fn has_pending_call(&self, id: u32) -> bool {
        self.pending_externals.contains_key(&CallId::new(id))
    }

    /// Registers a freshly created `ExternalFuture` for `call_id`.
    ///
    /// The scheduler inc_refs `future_id` so the entry stays alive between
    /// the yield to the host and the matching `resolve_future` / `fail_future`
    /// call, even if no awaiter holds a `Value::Ref` to it.
    pub fn add_pending_external(&mut self, call_id: CallId, future_id: HeapId, heap: &Heap) {
        heap.inc_ref(future_id);
        let prev = self.pending_externals.insert(call_id, future_id);
        debug_assert!(prev.is_none(), "add_pending_external: CallId already registered");
    }

    /// Removes and returns the `ExternalFuture` heap id for `call_id`, if any.
    ///
    /// The caller becomes responsible for the inc_ref previously held by the
    /// scheduler (typically dec_ref'd once the state transition is committed).
    pub fn take_pending_external(&mut self, call_id: CallId) -> Option<HeapId> {
        self.pending_externals.remove(&call_id)
    }

    /// Marks the current task as `Blocked` on the awaitable at `awaitable_id`.
    ///
    /// The task will be unblocked when the awaitable settles and its awaiter
    /// slot routes back here (`Awaiter::Task(task_id)` on either
    /// `ExternalFuture::Pending` or `AwaitedGather`).
    pub fn block_current_on(&mut self, awaitable_id: HeapId, heap: &Heap) {
        if let Some(task_id) = self.current_task {
            let task = self.get_task_mut(task_id);
            debug_assert!(matches!(task.state, TaskState::Ready(Ok(()))));
            heap.inc_ref(awaitable_id);
            task.state = TaskState::Blocked(awaitable_id);
        }
    }

    /// Returns all pending (unresolved) CallIds.
    pub fn pending_call_ids(&self) -> Vec<CallId> {
        self.pending_externals.keys().copied().collect()
    }

    /// Removes the queue entry when delivering directly to an exiting task's waiter.
    pub fn remove_from_ready_queue(&mut self, task_id: TaskId) {
        self.ready_queue.retain(|&id| id != task_id);
    }

    /// Release a protected gather's first error only after all lifetime obligations settle.
    pub fn finish_gather_barrier(&mut self, owner: TaskId) {
        if self.gather_externals.values().any(|task| *task == owner)
            || self
                .public_gather_waits
                .get(&owner)
                .is_some_and(|waits| !waits.is_empty())
        {
            return;
        }
        let drained = self.task(owner).is_some_and(|task| {
            task.group_body_waits.as_ref().map_or_else(
                || task.gather_children.is_empty(),
                |waits| waits.iter().all(|id| !self.tasks.contains_key(id)),
            )
        });
        let task = self.get_task_mut(owner);
        if task.pending_gather_error == GatherErrorDelivery::Draining && drained {
            task.group_body_waits = None;
            task.pending_gather_error = GatherErrorDelivery::Pending;
            self.ready_queue.push_back(owner);
        }
    }

    pub fn coroutine_task(&self, id: HeapId) -> Option<TaskId> {
        self.coroutine_to_task.get(&id).copied()
    }

    pub fn has_coroutine_task(&self, id: HeapId) -> bool {
        self.coroutine_to_task.contains_key(&id)
    }

    /// Spawns a new task from a coroutine, enforcing one-task-per-coroutine.
    ///
    /// Returns `None` if `coroutine_id` is already driving a task —
    /// caught here because cross-gather reuse can hit two spawns while
    /// both coroutine states are still `New`, so the state check in
    /// `await_coroutine` doesn't catch it. Callers translate `None`
    /// into a `RuntimeError: cannot reuse already awaited coroutine`.
    /// Both `coroutine_id` and the `GatherSlot` built from `gather_id` become
    /// **owning** references held by the new task; the matching `dec_ref`
    /// happens in [`Scheduler::cancel_task`]. It takes `gather_id` rather than
    /// a ready-made `Awaiter` so the `None` return above has nothing to unwind.
    pub fn spawn(&mut self, heap: &Heap, coroutine_id: HeapId, gather_id: Option<HeapId>) -> Option<TaskId> {
        if self.coroutine_to_task.contains_key(&coroutine_id) {
            return None;
        }

        let task_id = TaskId::new(self.next_task_id);
        self.next_task_id += 1;

        // Take ownership of the heap references — the task now holds an inc_ref'd
        // pointer to its coroutine and (if applicable) its enclosing gather.
        // The slot is keyed by the coroutine's own id, which is what
        // `resolve_child` looks up.
        heap.inc_ref(coroutine_id);
        let awaiter = gather_id.map(|gather| {
            heap.inc_ref(gather);
            Awaiter::GatherSlot {
                gather,
                source: coroutine_id,
            }
        });

        let mut task = Task::new(task_id, Some(coroutine_id), awaiter);
        task.run_owner = self
            .current_task()
            .and_then(|parent| parent.run_frame.map(|_| parent.id).or(parent.run_owner));
        self.tasks.insert(task_id, task);
        self.coroutine_to_task.insert(coroutine_id, task_id);
        self.ready_queue.push_back(task_id);

        Some(task_id)
    }

    /// Gets the next ready task from the queue.
    ///
    /// Returns `None` if no tasks are ready.
    pub fn next_ready_task(&mut self) -> Option<TaskId> {
        self.ready_queue.pop_front()
    }

    /// Wakes a blocked task with a value already on its stack, or an exception to raise.
    pub fn make_ready(&mut self, task_id: TaskId, result: RunResult<()>, heap: &mut Heap) {
        let task = self.get_task_mut(task_id);
        debug_assert!(matches!(task.state, TaskState::Blocked(_)));
        let old_state = mem::replace(&mut task.state, TaskState::Ready(result));
        old_state.drop_with(heap);
        self.ready_queue.push_back(task_id);
    }

    /// Takes the selected task's resume result exactly once, before executing its context.
    pub fn take_resume_result(&mut self, task_id: TaskId) -> RunResult<()> {
        match &mut self.get_task_mut(task_id).state {
            TaskState::Ready(result) => mem::replace(result, Ok(())),
            TaskState::Blocked(_) => panic!("cannot resume a blocked task"),
        }
    }

    /// Sets the current task.
    pub fn set_current_task(&mut self, task_id: Option<TaskId>) {
        self.current_task = task_id;
    }

    /// Removes a task and releases its saved context and heap references.
    /// Cancels tasks under any gather it is still blocked on, using an iterative walk
    /// so deeply nested gathers cannot overflow the native stack during teardown.
    pub fn cancel_task(&mut self, task_id: TaskId, heap: &mut HeapReader<'_>) {
        let mut pending: SmallVec<[TaskId; 4]> = smallvec![task_id];
        while let Some(task_id) = pending.pop() {
            self.cancel_one(task_id, heap, &mut pending);
        }
    }

    /// Cancels one task, queueing the tasks spawned under any gather it was
    /// blocked on for [`Scheduler::cancel_task`] to drain.
    ///
    /// Dropping this task ahead of the children it queued is sound: each owns
    /// an inc_ref on that same gather (see [`Scheduler::spawn`]).
    fn cancel_one(&mut self, task_id: TaskId, heap: &mut HeapReader<'_>, pending: &mut SmallVec<[TaskId; 4]>) {
        // No-op if the task has already been removed (idempotent — finalization
        // sites may iterate task ids that include already-cancelled siblings).
        let Some(mut task) = self.tasks.remove(&task_id) else {
            return;
        };

        // The VM must discard this task's loaded context before activating another task.
        if self.current_task == Some(task_id) {
            self.current_task = None;
        }

        if let Some(coroutine_id) = task.coroutine_id {
            self.coroutine_to_task.remove(&coroutine_id);
        }

        self.ready_queue.retain(|&id| id != task_id);
        self.public_gather_waits.remove(&task_id);

        pending.extend(task.gather_children.iter().copied());
        self.gather_externals.retain(|_, owner| *owner != task_id);

        // Blocked on a gather: queue the tasks spawned under it. An
        // external future needs no extra teardown.
        if let TaskState::Blocked(blocked_id) = task.state {
            self.queue_gather_tasks(blocked_id, heap, pending);
        }

        // Low-level abandonment is physical disposal, never guest cleanup.
        // Owned children cannot outlive a discarded owner and deliver to it later.
        for child in &task.children {
            if let HeapReadOutput::ExternalFuture(future) = heap.read(*child)
                && let Some(id) = future.get(heap).child.as_ref().and_then(|child| child.task)
            {
                pending.push(id);
            }
        }
        if let Some(id) = task.pending_queue.take() {
            let (value, queue) = finish_queue_operation(id, false, heap);
            value.drop_with(heap);
            self.pump_queue(queue, heap);
            heap.dec_ref(queue);
            heap.dec_ref(id);
        }
        task.drop_with(heap);
    }

    /// Queues every task spawned under the gather `root`, walking nested
    /// gathers iteratively.
    ///
    /// A gather item can itself be a gather (`gather(gather(coro()))`), whose
    /// tasks are just as orphaned as direct coroutine children if left in
    /// `self.tasks` — they would keep running and then deliver a result to the
    /// task cancelled here. External children *are* left alone: the owning
    /// `Awaiter::GatherSlot` anchors them.
    ///
    /// Must run while the cancelled task still holds its `Blocked` inc_ref on
    /// `root`, since the walk takes no references of its own: each nested
    /// gather is kept alive by its parent's `items`, and the parent in turn by
    /// the `Awaiter::GatherSlot` inc_ref that nested child holds.
    fn queue_gather_tasks(&self, root: HeapId, heap: &HeapReader<'_>, pending: &mut SmallVec<[TaskId; 4]>) {
        // Gathers nest as a tree — a gather may only be awaited once, so the
        // walk cannot revisit a node and terminates.
        let mut gathers: SmallVec<[HeapId; 4]> = smallvec![root];
        while let Some(gather_id) = gathers.pop() {
            // Coroutine and external children land here too; only gathers have
            // children of their own to walk.
            let HeapReadOutput::GatherFuture(gather) = heap.read(gather_id) else {
                continue;
            };
            if let Some(awaited) = gather.get(heap).as_awaited() {
                for child_id in awaited.pending_children.keys() {
                    match self.coroutine_to_task.get(child_id) {
                        Some(&task_id) => pending.push(task_id),
                        None => gathers.push(*child_id),
                    }
                }
            }
            drop(gather);
        }
    }

    /// Whether this task still exists and is waiting for an awaitable to settle.
    #[inline]
    pub fn is_blocked(&self, task_id: TaskId) -> bool {
        self.tasks
            .get(&task_id)
            .is_some_and(|task| matches!(task.state, TaskState::Blocked(_)))
    }

    #[cfg(feature = "test-hooks")]
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    #[cfg(test)]
    pub fn ownership_gather_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.tasks.values().map(|task| task.gather_children.len()).sum(),
            self.gather_externals.len(),
            self.tasks.values().map(|task| task.gather_errors.len()).sum(),
            self.tasks
                .values()
                .filter(|task| task.pending_gather_error == GatherErrorDelivery::Pending)
                .count(),
        )
    }

    #[cfg(test)]
    pub fn ownership_child_counts(&self) -> (usize, usize, usize) {
        (
            self.tasks.len(),
            self.tasks.values().map(|task| task.children.len()).sum(),
            self.tasks.values().filter(|task| task.child_result.is_some()).count(),
        )
    }

    /// Cleans up all scheduler resources: the pending-future inc_refs and
    /// every remaining task (via [`Scheduler::cancel_task`]).
    pub fn cleanup(&mut self, heap: &mut HeapReader<'_>) {
        // Release the inc_refs the scheduler holds on each pending future.
        for (_, future_id) in mem::take(&mut self.pending_externals) {
            heap.dec_ref(future_id);
        }
        let task_ids: Vec<TaskId> = self.tasks.keys().copied().collect();
        for task_id in task_ids {
            self.cancel_task(task_id, heap);
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
