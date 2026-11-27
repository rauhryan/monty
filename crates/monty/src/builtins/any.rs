//! Implementation of any on the VM's suspendable iteration protocol.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::RunResult,
    frozen::FrozenFunction,
};

/// Iterate on ordinary frames, including user iterators and host callbacks.
pub fn builtin_any(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("any", vm.heap)?;
    vm.call_frozen_exact(FrozenFunction::Any, [iterable])
}

/// Iterator methods may suspend; truth conversion follows ordinary VM rules.
pub(crate) const ANY_SOURCE: &str = r"
def __monty_any(iterable):
    for item in iterable:
        if item:
            return True
    return False
";
