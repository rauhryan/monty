//! Lazy filter state on ordinary, suspendable iterator-method frames.

use crate::{
    args::ArgValues,
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    exception_private::RunResult,
    frozen::FrozenFunction,
    types::BuiltinType,
    value::Value,
};

/// Acquire the source immediately; evaluate predicates only when advanced.
pub fn builtin_filter(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let (function, iterable) = args.get_two_args("filter", vm.heap)?;
    vm.call_frozen_exact(
        FrozenFunction::Filter,
        [
            function,
            iterable,
            Value::Builtin(Builtins::Type(BuiltinType::Iterator)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
        ],
    )
}

/// A failed predicate consumes its item without closing the filter iterator.
/// The active method frame owns the current item across suspension.
pub(crate) const FILTER_SOURCE: &str = r"
def __monty_filter(function, iterable, iter_, next_):
    class filter:
        def __init__(self, function, iterator):
            self.function = function
            self.iterator = iterator

        def __iter__(self):
            return self

        def __next__(self):
            while True:
                item = next_(self.iterator)
                if self.function is None:
                    keep = item
                else:
                    keep = self.function(item)
                if keep:
                    return item
                item = None
                keep = None

    return filter(function, iter_(iterable))
";
