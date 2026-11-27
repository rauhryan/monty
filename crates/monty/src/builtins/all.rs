//! Implementation of all on the VM's suspendable iteration protocol.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::RunResult,
    frozen::FrozenFunction,
};

/// Iterate on ordinary frames, including user iterators and host callbacks.
pub fn builtin_all(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("all", vm.heap)?;
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
