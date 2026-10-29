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
    module.set_attr(
        StaticStrings::SandboxAsk,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::Ask)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxEcho,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::Echo)),
        vm,
    );
    module.set_attr(
        StaticStrings::Random,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::Random)),
        vm,
    );
    module.set_attr(
        StaticStrings::Sample,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::Sample)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxDecision,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::Decision)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxDocumentCreate,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::DocumentCreate)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxDocumentRead,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::DocumentRead)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxDocumentEdit,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::DocumentEdit)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxPresentChoices,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::PresentChoices)),
        vm,
    );
    module.set_attr(
        StaticStrings::SandboxDisplayDocument,
        Value::ModuleFunction(ModuleFunctions::SandboxCapability(Capability::DisplayDocument)),
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

/// Platform contract names, not authority: every request is authorized by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Capability {
    Ask,
    Echo,
    Random,
    Sample,
    Decision,
    DocumentCreate,
    DocumentRead,
    DocumentEdit,
    PresentChoices,
    DisplayDocument,
}
impl Capability {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Echo => "echo",
            Self::Random => "random",
            Self::Sample => "sample",
            Self::Decision => "decision",
            Self::DocumentCreate => "document_create",
            Self::DocumentRead => "document_read",
            Self::DocumentEdit => "document_edit",
            Self::PresentChoices => "present_choices",
            Self::DisplayDocument => "display_document",
        }
    }
    fn attribute(self) -> StaticStrings {
        match self {
            Self::Ask => StaticStrings::SandboxAsk,
            Self::Echo => StaticStrings::SandboxEcho,
            Self::Random => StaticStrings::Random,
            Self::Sample => StaticStrings::Sample,
            Self::Decision => StaticStrings::SandboxDecision,
            Self::DocumentCreate => StaticStrings::SandboxDocumentCreate,
            Self::DocumentRead => StaticStrings::SandboxDocumentRead,
            Self::DocumentEdit => StaticStrings::SandboxDocumentEdit,
            Self::PresentChoices => StaticStrings::SandboxPresentChoices,
            Self::DisplayDocument => StaticStrings::SandboxDisplayDocument,
        }
    }
}

pub(super) fn call(capability: Capability, vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let request = args.get_one_arg(capability.name(), vm.heap)?;
    let name = Value::InternString(vm.interns.intern_static(capability.attribute()));
    let exchange = vm.heap.get_ext_function("sandbox.exchange");
    vm.frozen_awaitable(FrozenFunction::SandboxCapability, vec![name, request, exchange], true)
}
