//! Implementation of the `asyncio` module.
//!
//! Provides a minimal implementation of Python's `asyncio` module with:
//! - `run(coro)`: Runs a coroutine to completion, equivalent to `await coro`
//! - `gather(*awaitables)`: Collects coroutines for concurrent execution
//! - `sleep(delay, result=None)`: An awaitable governed by the session's `SleepMode`
//!
//! - `create_task(coro)`: Schedules a coroutine and returns a persistent Task handle
//!
//! Queue provides bounded/unbounded FIFO coroutine operations.
//! TaskGroup provides structured waiting and failure drain; other functions (`wait`, etc.) are not implemented.
//! The host acts as the event loop - Monty yields control when tasks are blocked.

use std::mem;

use monty_types::{OsFunctionCall, sleep_duration_saturating};
use num_traits::ToPrimitive;

use crate::{
    args::{ArgValues, FromArgs},
    asyncio::GatherFuture,
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::{Heap, HeapData, HeapId},
    heap_traits::DropGuard,
    intern::StaticStrings,
    modules::{
        ModuleFunctions,
        time::{HostSleep, host_sleep},
    },
    os_dispatch::PostConversionEffect,
    types::Module,
    value::{EitherStr, Value},
};

/// Async Functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum AsyncioFunctions {
    Gather,
    Run,
    Sleep,
    CreateTask,
    TaskGroup,
    GroupCreateTask,
    TaskDone,
    TaskCancelled,
    TaskCancel,
    TaskResult,
    TaskException,
    NativeSpawn,
    NativeScope,
    NativeCancel,
    NativeDrain,
    NativeQueue,
    NativeQueuePut,
    NativeQueueGet,
    Queue,
    QueuePut,
    QueueGet,
    QueueSize,
    QueueEmpty,
    QueueFull,
}

/// Creates the `asyncio` module and allocates it on the heap.
///
/// Public asyncio support includes Tasks, TaskGroup and Queue; private native child-scope
/// primitives are experimental authoring bindings, not additional asyncio parity.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Asyncio, vm.interns);

    module.set_attr(
        StaticStrings::Gather,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Gather)),
        vm,
    );
    module.set_attr(
        StaticStrings::Run,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Run)),
        vm,
    );
    module.set_attr(
        StaticStrings::Sleep,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Sleep)),
        vm,
    );

    module.set_attr(
        StaticStrings::CreateTask,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::CreateTask)),
        vm,
    );

    module.set_attr(
        StaticStrings::TaskGroup,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::TaskGroup)),
        vm,
    );
    module.set_attr(
        StaticStrings::AsyncioQueue,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Queue)),
        vm,
    );
    for (name, function) in [
        (StaticStrings::NativeScope, AsyncioFunctions::NativeScope),
        (StaticStrings::NativeSpawn, AsyncioFunctions::NativeSpawn),
        (StaticStrings::NativeCancel, AsyncioFunctions::NativeCancel),
        (StaticStrings::NativeDrain, AsyncioFunctions::NativeDrain),
        (StaticStrings::NativeQueue, AsyncioFunctions::NativeQueue),
        (StaticStrings::NativeQueuePut, AsyncioFunctions::NativeQueuePut),
        (StaticStrings::NativeQueueGet, AsyncioFunctions::NativeQueueGet),
    ] {
        module.set_attr(name, Value::ModuleFunction(ModuleFunctions::Asyncio(function)), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
pub(crate) fn call(vm: &mut VM<'_>, functions: AsyncioFunctions, args: ArgValues) -> RunResult<CallResult> {
    match functions {
        AsyncioFunctions::Queue => {
            let QueueArgs { maxsize } = QueueArgs::from_args(args, vm)?;
            defer_drop!(maxsize, vm);
            vm.new_public_queue(maxsize).map(CallResult::Value)
        }
        AsyncioFunctions::QueuePut => {
            let QueuePutArgs { queue, item } = QueuePutArgs::from_args(args, vm)?;
            defer_drop!(queue, vm);
            defer_drop!(item, vm);
            let operation = vm.new_queue_operation(queue, Some(item))?;
            vm.frozen_awaitable(FrozenFunction::QueueOperation, vec![operation], false)
                .map(CallResult::Value)
        }
        AsyncioFunctions::QueueGet => {
            let queue = args.get_one_arg("get", vm.heap)?;
            defer_drop!(queue, vm);
            let operation = vm.new_queue_operation(queue, None)?;
            vm.frozen_awaitable(FrozenFunction::QueueOperation, vec![operation], false)
                .map(CallResult::Value)
        }
        AsyncioFunctions::QueueSize | AsyncioFunctions::QueueEmpty | AsyncioFunctions::QueueFull => {
            let queue = args.get_one_arg("Queue inspection", vm.heap)?;
            defer_drop!(queue, vm);
            let Value::Ref(id) = queue else {
                return Err(ExcType::type_error("Queue receiver required"));
            };
            let HeapData::ExternalFuture(future) = vm.heap.get(*id) else {
                return Err(ExcType::type_error("Queue receiver required"));
            };
            let queue = future
                .queue
                .as_ref()
                .ok_or_else(|| ExcType::type_error("Queue receiver required"))?;
            Ok(CallResult::Value(match functions {
                AsyncioFunctions::QueueSize => {
                    Value::Int(i64::try_from(queue.values.len()).expect("queue length fits i64"))
                }
                AsyncioFunctions::QueueEmpty => Value::Bool(queue.values.is_empty()),
                _ => Value::Bool(queue.capacity > 0 && queue.values.len() >= queue.capacity),
            }))
        }

        AsyncioFunctions::TaskGroup => {
            args.check_zero_args("TaskGroup", vm.heap)?;
            Ok(CallResult::Value(vm.new_task_group()))
        }
        AsyncioFunctions::GroupCreateTask => {
            let (group, coro) = args.get_two_args("create_task", vm.heap)?;
            defer_drop!(group, vm);
            defer_drop!(coro, vm);
            let Value::Ref(id) = group else {
                return Err(ExcType::type_error("TaskGroup receiver required"));
            };
            vm.group_create_task(*id, coro).map(CallResult::Value)
        }

        AsyncioFunctions::TaskDone
        | AsyncioFunctions::TaskCancelled
        | AsyncioFunctions::TaskCancel
        | AsyncioFunctions::TaskResult
        | AsyncioFunctions::TaskException => {
            let value = args.get_one_arg("Task method", vm.heap)?;
            defer_drop!(value, vm);
            let Value::Ref(id) = value else {
                return Err(ExcType::type_error("Task method requires a Task receiver"));
            };
            if !matches!(vm.heap.get(*id), HeapData::ExternalFuture(future) if future.public_task.is_some()) {
                return Err(ExcType::type_error("Task method requires a Task receiver"));
            }
            let name = match functions {
                AsyncioFunctions::TaskDone => "done",
                AsyncioFunctions::TaskCancelled => "cancelled",
                AsyncioFunctions::TaskCancel => "cancel",
                AsyncioFunctions::TaskResult => "result",
                _ => "exception",
            };
            vm.call_public_task(*id, &EitherStr::Heap(name.to_owned()), ArgValues::Empty)
        }
        AsyncioFunctions::CreateTask => {
            let value = args.get_one_arg("create_task", vm.heap)?;
            defer_drop!(value, vm);
            vm.spawn_public_task(value).map(CallResult::Value)
        }
        AsyncioFunctions::Gather => gather(vm, args).map(CallResult::Value),
        AsyncioFunctions::Run => run(vm.heap, args),
        AsyncioFunctions::Sleep => sleep(vm, args),
        AsyncioFunctions::NativeQueue => {
            let capacity = args.get_one_arg("_queue", vm.heap)?;
            defer_drop!(capacity, vm);
            vm.new_guest_queue(capacity).map(CallResult::Value)
        }
        AsyncioFunctions::NativeQueueGet => {
            let queue = args.get_one_arg("_queue_get", vm.heap)?;
            defer_drop!(queue, vm);
            vm.new_queue_operation(queue, None).map(CallResult::Value)
        }
        AsyncioFunctions::NativeQueuePut => {
            let (queue, value) = args.get_two_args("_queue_put", vm.heap)?;
            defer_drop!(queue, vm);
            defer_drop!(value, vm);
            vm.new_queue_operation(queue, Some(value)).map(CallResult::Value)
        }
        AsyncioFunctions::NativeScope => {
            args.check_zero_args("_scope", vm.heap)?;
            vm.open_child_scope().map(|()| CallResult::Value(Value::None))
        }
        AsyncioFunctions::NativeSpawn => {
            let value = args.get_one_arg("_spawn", vm.heap)?;
            defer_drop!(value, vm);
            vm.spawn_owned_child(value).map(CallResult::Value)
        }
        AsyncioFunctions::NativeCancel => {
            let value = args.get_one_arg("_cancel", vm.heap)?;
            defer_drop!(value, vm);
            vm.cancel_owned_child(value, "guest")
                .map(|()| CallResult::Value(Value::None))
        }
        AsyncioFunctions::NativeDrain => {
            args.check_zero_args("_drain", vm.heap)?;
            vm.drain_owned_children().map(CallResult::Value)
        }
    }
}

/// Returns an awaitable producing `result`, retained by [`PostConversionEffect::SleepResult`].
/// Unlike CPython, waiting starts at the call. The host answers with a future
/// or inline; [`host_sleep`] determines the destination and delay.
/// `Zero` and zero-length system sleeps settle immediately. See `limitations/asyncio.md`.
fn sleep(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let SleepArgs { delay, result } = SleepArgs::from_args(args, vm)?;
    // `result` outlives `delay`: it moves into the awaitable once the delay is valid.
    let mut result_guard = DropGuard::new(result, vm);
    let delay = {
        let (_, vm) = result_guard.as_parts_mut();
        defer_drop!(delay, vm);
        let seconds = delay_seconds(delay, vm)?;
        // NaN is the one delay CPython refuses; the rest clamp.
        let delay = sleep_duration_saturating(seconds)
            .map_err(|_| ExcType::value_error("Invalid delay: NaN (not a number)"))?;
        host_sleep(vm, delay)
    };
    let (result, vm) = result_guard.into_parts();
    let call = match delay {
        Some(HostSleep::CallHost(delay)) => OsFunctionCall::AsyncSleep(delay),
        // Avoid a host round trip for a zero-length system sleep.
        Some(HostSleep::System(delay)) if !delay.is_zero() => OsFunctionCall::AsyncSystemSleep(delay),
        _ => return Ok(CallResult::Value(vm.settled_awaitable(result))),
    };
    Ok(CallResult::OsCallWithEffect {
        call,
        effect: PostConversionEffect::SleepResult { result }.into(),
    })
}

/// `asyncio.sleep(delay, result=None)` — a pure-Python `def` in CPython, so
/// both parameters bind by keyword and neither is type-checked while binding.
#[derive(FromArgs)]
#[from_args(name = "sleep", style = def)]
struct SleepArgs {
    #[from_args(static_string = "Delay")]
    delay: Value,
    #[from_args(static_string = "ResultArg", default = Value::None)]
    result: Value,
}

/// Reads `delay` as float seconds.
///
/// CPython never converts it: it compares `delay <= 0` and hands whatever is
/// left to the loop, so only real numbers work — an `__index__`-able class is
/// rejected here although `time.sleep()` accepts it — and a non-number fails as
/// an unsupported comparison rather than as a bad argument.
fn delay_seconds(delay: &Value, vm: &VM<'_>) -> RunResult<f64> {
    match delay {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        Value::Bool(b) => Ok(f64::from(*b)),
        _ => match delay.as_long_int(vm) {
            Some(n) => Ok(n.to_f64().unwrap_or(f64::INFINITY)),
            None => Err(ExcType::type_error_ordering("<=", &delay.py_type_name(vm), "int")),
        },
    }
}

/// Implementation of `asyncio.run(coro)`.
///
/// Runs a single coroutine to completion with an explicit VM-owned shutdown boundary.
/// Accepts exactly one positional argument (the coroutine) and no keyword arguments.
///
/// Returns `CallResult::AwaitValue` so the VM executes `exec_get_awaitable` on
/// the value, which handles validation that it's actually a coroutine/awaitable.
fn run(heap: &mut Heap, args: ArgValues) -> RunResult<CallResult> {
    let coroutine = args.get_one_arg("asyncio.run", heap)?;
    Ok(CallResult::AwaitValue(coroutine))
}

/// Implementation of `asyncio.gather(*awaitables)`.
///
/// Collects coroutines and external futures for concurrent execution. Does NOT
/// spawn tasks immediately - just validates and stores the references. Tasks are
/// spawned when the returned `GatherFuture` is awaited (in the `Await` opcode handler).
///
/// # Behavior when awaited
///
/// 1. Each coroutine is spawned as a separate Task
/// 2. External futures are tracked for resolution by the host
/// 3. The current task blocks until all items complete
/// 4. Results are collected in order and returned as a list
/// 5. On any task failure, sibling tasks are cancelled and the exception propagates
///
/// # Arguments
/// * `heap` - The heap for allocating the GatherFuture
/// * `args` - Variadic awaitable arguments (coroutines or external futures)
///
/// # Errors
/// Returns `TypeError` if any argument is not awaitable.
pub(crate) fn gather(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    // TODO: support keyword arguments (e.g. return_exceptions); for now any
    // kwarg is rejected up front by the macro's `kwargs_not_supported_yet`
    // flag with a `NotImplementedError: gather() does not yet support keyword
    // arguments` (CPython would have given a TypeError naming the bad kwarg).
    let GatherArgs { awaitables } = GatherArgs::from_args(args, vm)?;
    defer_drop_mut!(awaitables, vm);

    // Validate every argument before transferring any references to the gather,
    // so an invalid later argument needs no raw-HeapId rollback.
    for arg in awaitables.iter() {
        if !matches!(
            arg,
            Value::Ref(id)
                if matches!(
                    vm.heap.get(*id),
                    HeapData::Coroutine(_) | HeapData::ExternalFuture(_) | HeapData::GatherFuture(_) | HeapData::Generator(_)
                )
        ) {
            return Err(ExcType::type_error(
                "An asyncio.Future, a coroutine or an awaitable is required",
            ));
        }
    }

    // Keep the vector guarded through every fallible bridge compilation. Each
    // replacement transfers exactly one owned reference into a VM coroutine.
    for arg in awaitables.iter_mut() {
        if matches!(arg, Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Generator(_))) {
            let continuation = mem::replace(arg, Value::None);
            *arg = vm.host_continuation_coroutine(continuation)?;
        }
    }

    let items = awaitables
        .drain(..)
        .map(|arg| arg.into_ref_id().expect("validated gather awaitable is heap-backed"))
        .collect();
    let gather_future = GatherFuture::new(items);
    let id = vm.heap.allocate(HeapData::GatherFuture(Box::new(gather_future)));
    Ok(Value::Ref(id))
}

/// `asyncio.gather(*awaitables)` — variadic positional, no kwargs accepted.
///
/// `kwargs_not_supported_yet` rejects any kwarg with the macro's
/// "not yet implemented" `NotImplementedError`, replacing the previous
/// `TypeError: gather() takes no keyword arguments` from `into_pos_only`.
/// When CPython's `return_exceptions=False` is wired up this becomes a
/// regular `kw_only` slot and the flag goes away.
#[derive(FromArgs)]
#[from_args(name = "gather", kwargs_not_supported_yet)]
struct GatherArgs {
    #[from_args(varargs)]
    awaitables: Vec<Value>,
}

/// Public constructor supports the ordinary positional/keyword maxsize binding.
#[derive(FromArgs)]
#[from_args(name = "Queue", style = def)]
struct QueueArgs {
    #[from_args(static_string = "Maxsize", default = Value::Int(0))]
    maxsize: Value,
}

#[derive(FromArgs)]
#[from_args(name = "put", style = def)]
struct QueuePutArgs {
    #[from_args(pos_only)]
    queue: Value,
    item: Value,
}
