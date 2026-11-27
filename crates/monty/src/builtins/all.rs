//! Suspendable implementation of the all() builtin function.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    frozen::FrozenFunction,
    heap::HeapReadOutput,
    types::PyTrait,
    value::Value,
};

/// Consume an iterable on ordinary VM frames, preserving short-circuit evaluation.
pub fn builtin_all(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("all", vm.heap)?;
    if !matches!(
        iterable.read_heap(vm),
        Some(HeapReadOutput::Generator(_) | HeapReadOutput::Instance(_))
    ) {
        let iterator = iterable.into_py_iter(vm)?.require_synchronous(vm)?;
        defer_drop!(iterator, vm);
        let mut iterator = iterator.read(vm);
        while let Some(item) = iterator.py_next(vm)? {
            defer_drop!(item, vm);
            if !item.py_bool(vm)? {
                return Ok(CallResult::Value(Value::Bool(false)));
            }
        }
        return Ok(CallResult::Value(Value::Bool(true)));
    }
    vm.call_frozen_exact(FrozenFunction::All, [iterable])
}

/// Iterator methods may suspend; truth conversion follows ordinary VM rules.
pub(crate) const ALL_SOURCE: &str = r"
def __monty_all(iterable):
    for item in iterable:
        if not item:
            return False
    return True
";
