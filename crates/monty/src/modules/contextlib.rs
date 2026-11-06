//! Synchronous generator context managers implemented with ordinary guest frames.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, RunResult},
    frozen::{FrozenFunction, FrozenFunctions},
    heap::{HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    modules::ModuleFunctions,
    types::Module,
    value::Value,
};

/// Creates the bounded `contextlib` namespace.
pub(super) fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Contextlib, vm.interns);
    module.set_attr(
        StaticStrings::Contextmanager,
        Value::ModuleFunction(ModuleFunctions::Contextmanager),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

#[derive(FromArgs)]
#[from_args(name = "contextmanager", style = def)]
struct ContextmanagerArgs {
    func: Value,
}

/// Capture dependencies in closure cells, never in the user's globals.
pub(super) fn call(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ContextmanagerArgs { func } = ContextmanagerArgs::from_args(args, vm)?;
    vm.call_frozen_exact(
        FrozenFunction::Contextmanager,
        [
            func,
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
            Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
            Value::Builtin(Builtins::ExcType(ExcType::RuntimeError)),
            Value::Builtin(Builtins::ExcType(ExcType::BaseException)),
            Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::GeneratorStopCause)),
        ],
    )
}

/// Only the VM's PEP 479 conversion carries this owned cause.
pub(crate) fn stop_cause(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let (error, original) = args.get_two_args("generator_stop_cause", vm.heap)?;
    defer_drop!(error, vm);
    defer_drop!(original, vm);
    let matches = match (error.read_heap(vm), original) {
        (Some(HeapReadOutput::Exception(error)), Value::Ref(original)) => {
            matches!(&error.get(vm.heap).generator_stop_cause, Some(Value::Ref(cause)) if cause == original)
        }
        _ => false,
    };
    Ok(Value::Bool(matches))
}

/// CPython's one-yield protocol, including identity-sensitive suppression and PEP 479.
pub(crate) const CONTEXTMANAGER_SOURCE: &str = r#"
def __monty_contextmanager(func, next_, stop_, runtime_, base_, stop_cause):
    class _GeneratorContextManager:
        def __init__(self, func, args, kwargs):
            self.func = func
            self.args = args
            self.kwargs = kwargs
            self.gen = func(*args, **kwargs)

        def __enter__(self):
            self.func = self.args = self.kwargs = None
            try:
                return next_(self.gen)
            except stop_:
                raise runtime_("generator didn't yield") from None

        def __exit__(self, typ, value, traceback):
            if typ is None:
                try:
                    next_(self.gen)
                except stop_:
                    return False
                try:
                    raise runtime_("generator didn't stop")
                finally:
                    self.gen.close()
            if value is None:
                value = typ()
            try:
                self.gen.throw(value)
            except stop_ as error:
                return error is not value
            except runtime_ as error:
                if error is value or stop_cause(error, value):
                    return False
                raise
            except base_ as error:
                if error is not value:
                    raise
                return False
            try:
                raise runtime_("generator didn't stop after throw()")
            finally:
                self.gen.close()

        def __call__(self, decorated):
            def inner(*args, **kwargs):
                with _GeneratorContextManager(self.func, self.args, self.kwargs):
                    return decorated(*args, **kwargs)
            return inner

    def helper(*args, **kwargs):
        return _GeneratorContextManager(func, args, kwargs)
    return helper
"#;
