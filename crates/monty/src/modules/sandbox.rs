//! Supplied platform capabilities. These only yield structured host requests.
//! Authorization, effects and persistence belong to the host, never guest OS I/O.
use crate::{
    args::{ArgValues, FromArgs},
    bytecode::VM,
    exception_private::RunResult,
    frozen::FrozenFunction,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    modules::ModuleFunctions,
    types::Module,
    value::Value,
};

pub(super) fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Sandbox, vm.interns);
    module.set_attr(
        StaticStrings::SandboxReadFile,
        Value::ModuleFunction(ModuleFunctions::SandboxReadFile),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

#[derive(FromArgs)]
#[from_args(name = "read_file", style = def)]
struct ReadFileArgs {
    path: Value,
}

pub(super) fn read_file(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let ReadFileArgs { path } = ReadFileArgs::from_args(args, vm)?;
    let exchange = vm.heap.get_ext_function("sandbox.exchange");
    vm.frozen_awaitable(FrozenFunction::SandboxReadFile, vec![path, exchange], true)
}
