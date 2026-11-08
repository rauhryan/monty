//! Lazy zip state on ordinary, suspendable iterator-method frames.

use std::mem;

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, RunResult},
    frozen::FrozenFunction,
    heap::HeapData,
    types::{BuiltinType, List, PyTrait},
    value::Value,
};

/// Check strict truthiness before acquiring sources in argument order.
pub fn builtin_zip(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ZipArgs { iterables, strict } = ZipArgs::from_args(args, vm)?;
    defer_drop_mut!(iterables, vm);
    defer_drop!(strict, vm);
    let strict = strict.py_bool(vm)?;
    let sources = Value::Ref(vm.heap.allocate(HeapData::List(List::new(mem::take(iterables)))));
    vm.call_frozen_exact(
        FrozenFunction::Zip,
        [
            sources,
            Value::Bool(strict),
            Value::Builtin(Builtins::Type(BuiltinType::Iterator)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
            Value::Builtin(Builtins::Type(BuiltinType::Tuple)),
            Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
            Value::Builtin(Builtins::ExcType(ExcType::ValueError)),
            Value::Builtin(Builtins::Type(BuiltinType::Str)),
        ],
    )
}

/// Each call owns its partial row. Errors discard that row without closing zip.
/// Strict exhaustion is consumed before raising a length error, preserving exception context.
pub(crate) const ZIP_SOURCE: &str = r"
def __monty_zip(sources, strict, iter_, next_, tuple_, stop_, error_, str_):
    missing = []

    def length_error(index, relation):
        others = 'argument 1' if index == 1 else 'arguments 1-' + str_(index)
        return error_('zip() argument ' + str_(index + 1) + ' is ' + relation + ' than ' + others)

    class zip:
        def __init__(self, iterators):
            self.iterators = iterators

        def __iter__(self):
            return self

        def __next__(self):
            if not self.iterators:
                raise stop_
            items = []
            index = 0
            for iterator in self.iterators:
                if not strict:
                    items.append(next_(iterator))
                else:
                    item = next_(iterator, missing)
                    if item is missing:
                        if index:
                            raise length_error(index, 'shorter')
                        other_index = 0
                        for remaining in self.iterators:
                            if other_index and next_(remaining, missing) is not missing:
                                raise length_error(other_index, 'longer')
                            other_index += 1
                        raise stop_
                    items.append(item)
                index += 1
            return tuple_(items)

    iterators = []
    for source in sources:
        iterators.append(iter_(source))
    return zip(iterators)
";

/// Argument shape for `zip(*iterables, strict=False)`.
///
/// `strict` is held as a `Value` because CPython evaluates it via truthiness
/// (`py_bool`), not strict-type equality, so accepting any value is correct.
#[derive(FromArgs)]
#[from_args(name = "zip")]
struct ZipArgs {
    #[from_args(varargs)]
    iterables: Vec<Value>,
    #[from_args(default = Value::Bool(false))]
    strict: Value,
}
