//! Implementation of the map() builtin function.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::{DropWithContext, HeapData},
    types::List,
    value::Value,
};

/// Acquire sources eagerly and apply the callback only when advanced.
/// The Python 3.14 `strict` keyword remains unsupported.
pub fn builtin_map(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    // CPython's map() uses a bespoke arity message
    // (`map() must have at least two arguments.`) rather than the generic
    // "missing N required positional arguments" wording the macro would
    // otherwise produce. Pre-check before delegating to MapArgs so we
    // match byte-for-byte; cleanup is handled by the early-return drop.
    //
    // Only fire the arity check when no kwargs are present — otherwise
    // `map(abs, bogus=1)` would report the arity error when CPython reports
    // the unknown-kwarg error. Delegating to the macro produces the
    // correct `got an unexpected keyword argument` message instead.
    let kwargs_empty = match &args {
        ArgValues::Kwargs(kwargs) => kwargs.is_empty(),
        ArgValues::ArgsKargs { kwargs, .. } => kwargs.is_empty(),
        _ => true,
    };
    if args.count() < 2 && kwargs_empty {
        args.drop_with(vm.heap);
        return Err(ExcType::type_error_map_arity());
    }
    let MapArgs {
        function,
        first_iterable,
        extra_iterables,
    } = MapArgs::from_args(args, vm)?;
    let mut sources = vec![first_iterable];
    sources.extend(extra_iterables);
    let sources = Value::Ref(vm.heap.allocate(HeapData::List(List::new(sources))));
    vm.call_frozen_exact(
        FrozenFunction::Map,
        [
            function,
            sources,
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Zip)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
        ],
    )
}

/// Ordinary method frames allow retry after source or callback errors.
pub(crate) const MAP_SOURCE: &str = r"
def __monty_map(function, sources, zip_, next_):
    class map:
        def __init__(self, function, rows):
            self.function = function
            self.rows = rows

        def __iter__(self):
            return self

        def __next__(self):
            return self.function(*next_(self.rows))

    return map(function, zip_(*sources))
";

/// Argument shape for `map(function, iterable, *iterables)`.
///
/// `function` and the first `iterable` are required; any further iterables
/// are collected by `extra_iterables`. `map` doesn't accept kwargs, so the
/// macro's default unknown-kwarg error path is exactly what we want.
#[derive(FromArgs)]
#[from_args(name = "map")]
struct MapArgs {
    #[from_args(pos_only)]
    function: Value,
    #[from_args(pos_only)]
    first_iterable: Value,
    #[from_args(varargs)]
    extra_iterables: Vec<Value>,
}
