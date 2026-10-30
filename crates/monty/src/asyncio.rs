//! Async/await support types for Monty.
//!
//! This module contains all async-related types including coroutines, futures,
//! and task identifiers. The host acts as the event loop - external function
//! calls return `ExternalFuture` objects that can be awaited.

use std::collections::VecDeque;

use ahash::AHashMap;
use smallvec::SmallVec;

use crate::{
    exception_private::RunError,
    heap::{ContainsHeap, DropWithContext, HeapId},
    intern::FunctionId,
    value::Value,
};

/// Unique identifier for external function calls.
///
/// Sequential integers allocated by the scheduler. Used to correlate
/// external function calls with their results when the host resolves them.
/// The counter always increments, even for sync resolution, to keep IDs unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct CallId(u32);

impl CallId {
    /// Creates a new CallId from a raw value.
    #[inline]
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the raw u32 value.
    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Unique identifier for an async task.
///
/// Sequential integers allocated by the scheduler. Task 0 is always the main task
/// which uses the VM's stack/frames directly. Spawned tasks (1+) store their own context,
/// hence `TaskId::default()` is the main task.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskId(u32);

impl TaskId {
    /// Creates a new TaskId from a raw value.
    #[inline]
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns true if this is the main task (task 0).
    #[inline]
    pub fn is_main(self) -> bool {
        self.0 == 0
    }
}

/// Coroutine execution state (single-shot semantics).
///
/// Coroutines in Monty follow single-shot semantics - they can only be awaited once.
/// This differs from Python generators which can be resumed multiple times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CoroutineState {
    /// Coroutine has been created but not yet awaited.
    New,
    /// Coroutine is currently executing (has been awaited).
    Running,
    /// Coroutine has finished execution.
    Completed,
}

/// A coroutine object representing an async function call result.
///
/// Created when an `async def` function is called. Argument binding happens at call time;
/// awaiting the coroutine starts execution. Coroutines use single-shot semantics -
/// they can only be awaited once.
///
/// # Namespace Layout
///
/// The `namespace` vector is pre-sized to match the function's namespace size and contains:
/// ```text
/// [params...][cell_vars...][free_vars...][locals...]
/// ```
/// - Parameter slots are filled with bound argument values at call time
/// - Cell/free var slots contain `Value::Ref` to captured cells
/// - Local slots start as `Value::Undefined`
///
/// When the coroutine is awaited, these values are pushed onto the VM's stack
/// as inline locals, and a new frame is pushed to execute the async function body.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Coroutine {
    /// The async function to execute.
    pub func_id: FunctionId,
    /// Pre-bound namespace values (sized to function namespace).
    /// Contains bound parameters, captured cells, and uninitialized locals.
    pub namespace: Vec<Value>,
    /// Current execution state.
    pub state: CoroutineState,
    /// Owned reference to the `exec()` / `eval()` globals dict the async
    /// function was defined under; `None` when its globals are module slots.
    pub globals: Option<HeapId>,
}
impl Coroutine {
    /// Creates a new coroutine for an async function call.
    ///
    /// # Arguments
    /// * `func_id` - The async function to execute
    /// * `namespace` - Pre-bound namespace with parameters and captured variables
    /// * `globals` - The function's globals dict, already inc_ref'd for this coroutine
    pub fn new(func_id: FunctionId, namespace: Vec<Value>, globals: Option<HeapId>) -> Self {
        Self {
            func_id,
            namespace,
            state: CoroutineState::New,
            globals,
        }
    }
}

/// An external future driven by the host.
///
/// Created when the host returns `ExtFunctionResult::Future(call_id)` in
/// response to a function call yield. The future starts in `Pending`, and
/// transitions to `Resolved` (host returned a value) or `Failed` (host
/// returned an error) when [`VM::resolve_future`] / [`VM::fail_future`]
/// fires.
///
/// # Re-await semantics
///
/// `Resolved` / `Failed` futures can be awaited any number of times — each
/// await yields a clone of the cached value or replays the cached exception,
/// matching CPython's Future semantics. `Pending` futures still support only
/// a single in-flight awaiter (the `awaiter: Option<Awaiter>` slot);
/// public Tasks instead keep independent join routes in `public_task`.
/// Other pending external futures retain the single-waiter restriction.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ExternalFuture {
    /// Synthetic group __aenter__ result, before the exit handler is established.
    /// Ordinary settled futures and public Tasks never acquire this protocol role.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub protected_group_entry: bool,
    /// Public task outcome and independent pending join routes.
    #[serde(default)]
    pub public_task: Option<PublicTask>,
    #[serde(default)]
    pub task_group: Option<TaskGroup>,
    /// Lexical AnyIO scope carrier; no host-settleable registration.
    #[serde(default)]
    pub cancel_scope: Option<CancelScope>,
    /// Experimental internal queue token or single-delivery queue operation.
    #[serde(default)]
    pub queue: Option<GuestQueue>,
    #[serde(default)]
    pub queue_op: Option<QueueOperation>,
    /// Internal child identity, never registered as a host-settleable call.
    #[serde(default)]
    pub child: Option<ChildIdentity>,
    /// Host-side identifier. Used by the scheduler's `CallId -> HeapId`
    /// index so host resolutions can find the heap entry.
    pub call_id: CallId,
    /// Current state.
    pub state: ExternalFutureState,
    /// `asyncio.sleep(delay, result)`: the value to resolve with in place of
    /// the host's, which is only the wake-up signal. Owned; taken on
    /// resolution and released on failure or when the entry is freed.
    /// Internal child futures instead own their scope token here; they are
    /// never registered for host settlement, so it lives as long as the handle.
    pub sleep_result: Option<Value>,
}

/// A public Task has scheduler-owned lifetime, independent of its creator's scope.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PublicTask {
    pub task: Option<TaskId>,
    pub cancelled: bool,
    pub waiters: Vec<Awaiter>,
}

/// Admission and exit are single-use transitions, persisted across cleanup waits.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum TaskGroupState {
    Created,
    Active,
    Exiting,
    Finished,
}

/// Public structured group. The owner task roots active groups; groups root outcomes.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskGroup {
    /// Supplied AnyIO method surface; old asyncio groups keep their API.
    #[serde(default)]
    pub anyio: bool,
    /// New groups own a lexical scope; old active groups retain absent roles.
    #[serde(default)]
    pub cancel_scope: Option<HeapId>,
    #[serde(default)]
    pub cancel_suppressed: bool,
    pub owner: Option<TaskId>,
    pub state: TaskGroupState,
    pub children: Vec<HeapId>,
    pub body_error: Option<Value>,
    pub interrupt: Option<Value>,
    pub failure_pending: bool,
    pub externals: Vec<CallId>,
    pub errors: Vec<Value>,
}

/// Task/run-owned single-use lexical scope. The signal is an owned exact
/// exception identity, independent of native host cancellation and its latch.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct CancelScope {
    pub id: u64,
    pub owner: TaskId,
    pub run: u64,
    #[serde(flatten)]
    pub state: CancelScopeState,
    pub shield: bool,
    pub cancel_called: bool,
    pub cancelled_caught: bool,
    pub signal: Option<Value>,
}

/// A scope has one lexical lifetime. The wire adapter retains the exact v35
/// entered/exited map fields already written by genuine current fixtures.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "CancelScopeStateWire", into = "CancelScopeStateWire")]
pub(crate) enum CancelScopeState {
    Created,
    Active,
    Exited,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CancelScopeStateWire {
    entered: bool,
    exited: bool,
}

impl From<CancelScopeStateWire> for CancelScopeState {
    fn from(state: CancelScopeStateWire) -> Self {
        if state.exited {
            Self::Exited
        } else if state.entered {
            Self::Active
        } else {
            Self::Created
        }
    }
}

impl From<CancelScopeState> for CancelScopeStateWire {
    fn from(state: CancelScopeState) -> Self {
        Self {
            entered: state != CancelScopeState::Created,
            exited: state == CancelScopeState::Exited,
        }
    }
}

/// Bounded FIFO storage. Waiter IDs own references; reserved borrows a waiter.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GuestQueue {
    pub capacity: usize,
    /// Public maxsize (including negative unbounded sizes); absent on private/old tokens.
    #[serde(default)]
    pub public_maxsize: Option<i64>,
    pub values: VecDeque<Value>,
    pub getters: VecDeque<HeapId>,
    pub putters: VecDeque<HeapId>,
    pub reserved: Option<HeapId>,
}

/// An operation owns its queue and optional put value until delivery/cancellation.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct QueueOperation {
    pub queue: Option<HeapId>,
    pub put: Option<Value>,
    pub started: bool,
    pub task: Option<TaskId>,
}

impl ExternalFuture {
    /// Release queue edges and public Task join routes; mark Values consumed.
    pub(crate) fn queue_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        if let Some(task) = &self.public_task {
            for waiter in &task.waiters {
                if let Awaiter::GatherSlot { gather, .. } = waiter {
                    stack.push(*gather);
                }
            }
        }
        if let Some(scope) = &mut self.cancel_scope
            && let Some(signal) = &mut scope.signal
        {
            signal.py_dec_ref_ids(stack);
        }
        if let Some(group) = &mut self.task_group {
            stack.extend(group.cancel_scope);
            stack.extend(group.children.iter().copied());
            for value in &mut group.errors {
                value.py_dec_ref_ids(stack);
            }
            for value in [&mut group.body_error, &mut group.interrupt].into_iter().flatten() {
                value.py_dec_ref_ids(stack);
            }
        }
        if let Some(q) = &mut self.queue {
            for value in &mut q.values {
                value.py_dec_ref_ids(stack);
            }
            stack.extend(q.getters.iter().chain(&q.putters).copied());
        }
        if let Some(op) = &mut self.queue_op {
            if let Some(id) = op.queue {
                stack.push(id);
            }
            if let Some(value) = &mut op.put {
                value.py_dec_ref_ids(stack);
            }
        }
    }

    /// Enumerate queue edges and public Task join routes for cycle tracing.
    pub(crate) fn queue_owned_ids(&self, mut visit: impl FnMut(HeapId)) {
        if let Some(task) = &self.public_task {
            for waiter in &task.waiters {
                if let Awaiter::GatherSlot { gather, .. } = waiter {
                    visit(*gather);
                }
            }
        }
        if let Some(scope) = &self.cancel_scope
            && let Some(Value::Ref(id)) = &scope.signal
        {
            visit(*id);
        }
        if let Some(group) = &self.task_group {
            if let Some(id) = group.cancel_scope {
                visit(id);
            }
            for value in &group.errors {
                if let Value::Ref(id) = value {
                    visit(*id);
                }
            }
            for id in &group.children {
                visit(*id);
            }
            for value in [&group.body_error, &group.interrupt].into_iter().flatten() {
                if let Value::Ref(id) = value {
                    visit(*id);
                }
            }
        }
        if let Some(q) = &self.queue {
            for value in &q.values {
                if let Value::Ref(id) = value {
                    visit(*id);
                }
            }
            for id in q.getters.iter().chain(&q.putters) {
                visit(*id);
            }
        }
        if let Some(op) = &self.queue_op {
            if let Some(id) = op.queue {
                visit(id);
            }
            if let Some(Value::Ref(id)) = &op.put {
                visit(*id);
            }
        }
    }
}

/// Plain identity metadata; outcomes remain owned by the future state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChildIdentity {
    pub owner: TaskId,
    /// Borrowed identity; the future owns this token through `sleep_result`.
    pub scope: HeapId,
    pub task: Option<TaskId>,
    pub consumed: bool,
    pub cancelled: bool,
}

/// State machine for [`ExternalFuture`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum ExternalFutureState {
    /// Awaiting host resolution. `awaiter` is the downstream that should be
    /// notified on resolution; `None` until someone awaits, `Some(...)` for
    /// the lifetime of the await.
    Pending { awaiter: Option<Awaiter> },
    /// Resolved with a value. Re-awaits clone this Value.
    Resolved(Value),
    /// Rejected with an error. Re-awaits replay (clone) this error.
    Failed(RunError),
}

impl ExternalFuture {
    /// Creates a new `ExternalFuture` in the `Pending` state with no awaiter.
    pub fn new_pending(call_id: CallId, sleep_result: Option<Value>) -> Self {
        Self {
            call_id,
            protected_group_entry: false,
            public_task: None,
            task_group: None,
            cancel_scope: None,
            child: None,
            queue: None,
            queue_op: None,
            state: ExternalFutureState::Pending { awaiter: None },
            sleep_result,
        }
    }
}

/// Where the result/error of a completing awaitable should be routed.
///
/// Stored as the "downstream" of an awaitable.
///
/// - `Task` wakes the named task by setting it `Ready` and pushing the value
///   (or routing the error through its frame's exception handler). `TaskId`
///   is just a scheduler-side identifier, not a heap reference, so the
///   variant owns no inc_ref.
/// - `GatherSlot` fans the value into the gather at `gather`, looking up the
///   slot indices it should fill via the awaitable's own `HeapId` (`source`).
///   The wrapper **owns an inc_ref on `gather`**: storing the awaiter on an
///   awaitable keeps the gather alive for the in-flight window, so the
///   awaitable's resolution path can dispatch to it safely without
///   additional cleanup elsewhere. Drop the owned `Awaiter` via
///   [`DropWithContext`] (and clone via [`Self::clone_with_heap`]) so the
///   `gather` ref count stays balanced.
///
/// An awaitable whose downstream has already settled keeps its `Awaiter`
/// pointing at it; the value is discarded where it is delivered rather than
/// the link being severed here. See [`GatherState`].
///
/// Not `Copy` / `Clone` on purpose — the inc_ref discipline requires every
/// duplication to go through `clone_with_heap` and every discard to go
/// through `drop_with`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Awaiter {
    Task(TaskId),
    GatherSlot { gather: HeapId, source: HeapId },
}

impl<C: ContainsHeap> DropWithContext<C> for Awaiter {
    fn drop_with(self, heap: &mut C) {
        if let Self::GatherSlot { gather, .. } = self {
            heap.heap_mut().dec_ref(gather);
        }
    }
}

/// A gather() result tracking multiple coroutines/tasks and external futures.
///
/// Created by `asyncio.gather(*awaitables)`. Does NOT spawn tasks immediately -
/// tasks are spawned when the GatherFuture is awaited in Await.
///
/// # Lifecycle
///
/// The lifecycle is encoded in [`GatherState`]:
///
/// 1. **`Pending`** — created by `gather(coro1, coro2, ...)` but not yet awaited.
///    Only `items` carries data; the per-await bookkeeping does not yet exist.
/// 2. **`Awaited(AwaitedGather)`** — entered by the `Await` opcode. Spawned task
///    ids, the waiter, the per-slot results, and any external futures still
///    being waited on all live inside the [`AwaitedGather`] payload. Tasks and
///    external resolutions write into `results` slots while in this state.
/// 3. **`Completed(list_id)`** — all children completed successfully. The
///    `list_id` is an inc_ref'd `HeapData::List` holding the gathered results;
///    re-awaiting the gather returns this same list, matching CPython's
///    behavior of caching a Future's result.
/// 4. **`Failed(error)`** — a child task or external future raised. The error
///    was propagated to the original waiter on first await, and is cached here
///    so re-awaits re-raise the same exception (again matching CPython).
///
/// Encoding the phases as a `match`-able enum lets every site that touches a
/// gather state-transition explicitly, instead of inferring "have we been
/// awaited?" / "are we done?" from emptiness checks across several `Vec`s.
///
/// # Re-await semantics
///
/// `Completed` and `Failed` gathers can be awaited any number of times — each
/// await yields the same cached result or exception. Re-awaiting a gather that
/// is still in `Awaited` state (in-flight, the original waiter has not finished
/// driving it to completion) is currently rejected; supporting that would
/// require a list of waiters and is left as future work.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GatherFuture {
    /// Heap ids of items to gather. Each id points to an awaitable
    /// `HeapData` entry (currently `Coroutine` or `ExternalFuture`); the
    /// kind is recovered with `heap.read(id)` at gather-await time.
    ///
    /// Set once at construction and never mutated. The gather inc_refs each
    /// id and is the owner until drop, so GC must always walk this vector
    /// regardless of `state`.
    pub items: Vec<HeapId>,
    /// Phase of the gather lifecycle. See [`GatherState`].
    pub state: GatherState,
}

/// Lifecycle phase of a [`GatherFuture`].
///
/// See the `GatherFuture` docs for the transition rules.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum GatherState {
    /// Created but never awaited. No spawned tasks, no results, no waiter.
    Pending,
    /// Currently being awaited. `AwaitedGather` carries every per-await field.
    Awaited(AwaitedGather),
    /// All children completed successfully. Re-awaiting the gather inc_refs this
    /// value and returns it.
    Completed(Value),
    /// A child task failed (or an external future was rejected). The error
    /// is cached so subsequent awaits re-raise its identity. Clone with heap
    /// context when transitioning into this state.
    Failed(RunError),
}

/// Children of a gather that are still in flight → the slots each fills in the
/// gather's `results`.
///
/// Keyed by the child's own `HeapId` (coroutine, external future or nested
/// gather), which is how every resolution site addresses its parent's slots.
/// Duplicates from `gather(c, c)` share one entry with several indices.
pub(crate) type PendingChildren = AHashMap<HeapId, SmallVec<[usize; 1]>>;

/// Per-await bookkeeping for a [`GatherFuture`] in the `Awaited` phase.
///
/// All fields are populated when the gather is first awaited (in
/// `await_gather_future`) and progressively consumed as children resolve.
///
/// The gather is the single source of truth for "what awaitables I'm waiting
/// on and where their values go". Both maps follow the same lifecycle: each
/// entry is removed as the corresponding child resolves, and the gather is
/// done when both maps are empty.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct AwaitedGather {
    /// Downstream the gather should notify on completion. `Awaiter::Task(t)`
    /// is the user-level `await gather` case (waker is the task that ran
    /// the `await`); `Awaiter::GatherSlot { .. }` is the nested-gather case
    /// (this gather is an item of an outer gather, and its result list
    /// fans into the outer's slot). Single-waiter for now; the multi-waiter
    /// form is a planned follow-up.
    pub awaiter: Awaiter,
    /// Children → slots they fill in `results`, see [`PendingChildren`].
    ///
    /// Entries are removed as the corresponding child resolves. The gather
    /// is done when this map is empty.
    pub pending_children: PendingChildren,
    /// Results from each gather item, in order. Indices align with
    /// `GatherFuture::items`. Filled as tasks complete and externals resolve.
    pub results: Vec<Option<Value>>,
}

impl GatherFuture {
    /// Creates a new GatherFuture with the given item heap ids.
    ///
    /// # Arguments
    /// * `items` - Heap ids of awaitables (coroutines or external futures)
    pub fn new(items: Vec<HeapId>) -> Self {
        Self {
            items,
            state: GatherState::Pending,
        }
    }

    /// Returns the number of items to gather.
    #[inline]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Returns the per-await bookkeeping if the gather is in the `Awaited`
    /// phase. Convenience for read-only inspection sites that don't need to
    /// distinguish the other phases from one another.
    #[inline]
    pub fn as_awaited(&self) -> Option<&AwaitedGather> {
        match &self.state {
            GatherState::Awaited(awaited) => Some(awaited),
            GatherState::Pending | GatherState::Completed(_) | GatherState::Failed(_) => None,
        }
    }

    /// Mutable counterpart to [`Self::as_awaited`].
    #[inline]
    pub fn as_awaited_mut(&mut self) -> Option<&mut AwaitedGather> {
        match &mut self.state {
            GatherState::Awaited(awaited) => Some(awaited),
            GatherState::Pending | GatherState::Completed(_) | GatherState::Failed(_) => None,
        }
    }
}
