//! Context managers implemented with ordinary guest frames.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, RunResult},
    frozen::FrozenFunction,
    heap::{HeapData, HeapId},
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
    module.set_attr(
        StaticStrings::Closing,
        Value::ModuleFunction(ModuleFunctions::Closing),
        vm,
    );
    module.set_attr(
        StaticStrings::Asynccontextmanager,
        Value::ModuleFunction(ModuleFunctions::Asynccontextmanager),
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
        ],
    )
}

/// CPython's one-yield protocol, including identity-sensitive suppression and PEP 479.
pub(crate) const CONTEXTMANAGER_SOURCE: &str = r#"
def __monty_contextmanager(func, next_, stop_, runtime_, base_):
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
                if error is value or (typ is stop_ and error.__cause__ is value):
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

#[derive(FromArgs)]
#[from_args(name = "closing", style = def)]
struct ClosingArgs {
    thing: Value,
}

/// The close call executes on ordinary frames, including host suspension.
pub(super) fn closing(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ClosingArgs { thing } = ClosingArgs::from_args(args, vm)?;
    vm.call_frozen_exact(FrozenFunction::Closing, [thing])
}

pub(crate) const CLOSING_SOURCE: &str = r"
def __monty_closing(thing):
    class closing:
        def __init__(self, thing):
            self.thing = thing

        def __enter__(self):
            return self.thing

        def __exit__(self, *exc_info):
            self.thing.close()
    return closing(thing)
";

#[derive(FromArgs)]
#[from_args(name = "asynccontextmanager", style = def)]
struct AsynccontextmanagerArgs {
    func: Value,
}

/// Awaited work in the generator runs on the caller task's ordinary frames.
pub(super) fn asynccontextmanager(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let AsynccontextmanagerArgs { func } = AsynccontextmanagerArgs::from_args(args, vm)?;
    vm.call_frozen_exact(
        FrozenFunction::Asynccontextmanager,
        [
            func,
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
            Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
            Value::Builtin(Builtins::ExcType(ExcType::RuntimeError)),
            Value::Builtin(Builtins::ExcType(ExcType::BaseException)),
        ],
    )
}

/// The async protocol uses the same generator resume/throw/close routes as contextmanager.
pub(crate) const ASYNCCONTEXTMANAGER_SOURCE: &str = r#"
def __monty_asynccontextmanager(func, next_, stop_, runtime_, base_):
    class _AsyncGeneratorContextManager:
        def __init__(self, func, args, kwargs):
            self.func = func
            self.args = args
            self.kwargs = kwargs
            self.gen = func(*args, **kwargs)

        async def __aenter__(self):
            self.func = self.args = self.kwargs = None
            try:
                return next_(self.gen)
            except stop_:
                raise runtime_("generator didn't yield") from None

        async def __aexit__(self, typ, value, traceback):
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
                if error is value or (typ is stop_ and error.__cause__ is value):
                    return False
                raise
            except base_ as error:
                if error is not value:
                    raise
                return False
            try:
                raise runtime_("generator didn't stop after athrow()")
            finally:
                self.gen.close()

        def __call__(self, decorated):
            async def inner(*args, **kwargs):
                async with _AsyncGeneratorContextManager(self.func, self.args, self.kwargs):
                    return await decorated(*args, **kwargs)
            return inner

    def helper(*args, **kwargs):
        return _AsyncGeneratorContextManager(func, args, kwargs)
    return helper
"#;
