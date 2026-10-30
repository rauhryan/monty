//! Bounded supplied AnyIO authoring slice over the public asyncio scheduler.
//! Lexical cancellation scopes use task/run-owned native scheduler ancestry.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, RunResult},
    frozen::FrozenFunction,
    heap::{HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    modules::{ModuleFunctions, asyncio::AsyncioFunctions},
    types::{Module, PyTrait, allocate_tuple},
    value::Value,
};

/// Named identities persist in captured module values and frozen frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum AnyioFunctions {
    CreateTaskGroup,
    StartSoon,
    CancelScope,
    ScopeCancel,
    ScopeEnter,
    ScopeExit,
    GetCancelledExcClass,
}

/// Supply the bounded group and lexical scope surface.
pub(crate) fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Anyio, vm.interns);
    for (name, function) in [
        (StaticStrings::CreateTaskGroup, AnyioFunctions::CreateTaskGroup),
        (StaticStrings::CancelScope, AnyioFunctions::CancelScope),
        (
            StaticStrings::GetCancelledExcClass,
            AnyioFunctions::GetCancelledExcClass,
        ),
    ] {
        module.set_attr(name, Value::ModuleFunction(ModuleFunctions::Anyio(function)), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Bind both direct and extracted methods without retaining borrowed arguments.
#[derive(FromArgs)]
#[from_args(name = "start_soon", style = def)]
struct StartSoonArgs {
    #[from_args(pos_only, static_string = "Group")]
    group: Value,
    #[from_args(static_string = "Func")]
    func: Value,
    #[from_args(varargs)]
    args: Vec<Value>,
}

/// Bounded constructor: deadlines and writable shield are later work.
#[derive(FromArgs)]
#[from_args(name = "CancelScope", style = def)]
struct ScopeArgs {
    #[from_args(kw_only, default = Value::Bool(false))]
    shield: Value,
}

#[derive(FromArgs)]
#[from_args(name = "__exit__", style = def)]
struct ScopeExitArgs {
    #[from_args(pos_only)]
    scope: Value,
    #[from_args(pos_only)]
    exc_type: Value,
    #[from_args(pos_only)]
    exc_value: Value,
    #[from_args(pos_only)]
    traceback: Value,
}

/// Dispatch authoring calls into native group and lexical scope state.
pub(crate) fn call(vm: &mut VM<'_>, function: AnyioFunctions, args: ArgValues) -> RunResult<CallResult> {
    match function {
        AnyioFunctions::CreateTaskGroup => {
            args.check_zero_args("create_task_group", vm.heap)?;
            let value = vm.new_task_group();
            let Value::Ref(id) = value else { unreachable!() };
            let HeapReadOutput::ExternalFuture(future) = vm.heap.read(id) else {
                unreachable!()
            };
            future.get_mut(vm.heap).task_group.as_mut().expect("new group").anyio = true;
            Ok(CallResult::Value(value))
        }
        AnyioFunctions::StartSoon => {
            let StartSoonArgs { group, func, args } = StartSoonArgs::from_args(args, vm)?;
            // Binding owns all incoming values. The tuple and frozen frame take
            // those roots even if admission or the user callable later fails.
            let args = allocate_tuple(args.into_iter().collect(), vm.heap);
            vm.call_frozen_exact(
                FrozenFunction::AnyioStartSoon,
                [
                    group,
                    func,
                    args,
                    Value::ModuleFunction(ModuleFunctions::AnyioCheckGroup),
                    Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::GroupCreateTask)),
                ],
            )
        }
        AnyioFunctions::CancelScope => {
            let ScopeArgs { shield } = ScopeArgs::from_args(args, vm)?;
            defer_drop!(shield, vm);
            let shield = shield.py_bool(vm)?;
            Ok(CallResult::Value(vm.new_anyio_scope(shield)?))
        }
        AnyioFunctions::ScopeEnter => {
            let scope = args.get_one_arg("__enter__", vm.heap)?;
            defer_drop!(scope, vm);
            let Value::Ref(id) = scope else { unreachable!() };
            vm.anyio_scope_context(*id, None, true)
        }
        AnyioFunctions::ScopeExit => {
            let ScopeExitArgs {
                scope,
                exc_type,
                exc_value,
                traceback,
            } = ScopeExitArgs::from_args(args, vm)?;
            defer_drop!(scope, vm);
            defer_drop!(exc_type, vm);
            defer_drop!(exc_value, vm);
            defer_drop!(traceback, vm);
            let Value::Ref(id) = scope else { unreachable!() };
            let exc = if let Value::Ref(id) = exc_value {
                Some(*id)
            } else {
                None
            };
            vm.anyio_scope_context(*id, exc, false)
        }
        AnyioFunctions::ScopeCancel => {
            let scope = args.get_one_arg("cancel", vm.heap)?;
            defer_drop!(scope, vm);
            let Value::Ref(id) = scope else { unreachable!() };
            vm.cancel_anyio_scope(*id)?;
            Ok(CallResult::Value(Value::None))
        }
        AnyioFunctions::GetCancelledExcClass => {
            args.check_zero_args("get_cancelled_exc_class", vm.heap)?;
            Ok(CallResult::Value(Value::Builtin(Builtins::ExcType(
                ExcType::ValueError,
            ))))
        }
    }
}

/// Admission is checked before calling authored code, including bound methods.
pub(crate) fn check_group(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let group = args.get_one_arg("AnyIO group admission", vm.heap)?;
    defer_drop!(group, vm);
    let Value::Ref(id) = group else {
        unreachable!("frozen group receiver")
    };
    vm.check_group_admission(*id)?;
    Ok(CallResult::Value(Value::None))
}
