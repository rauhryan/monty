//! Implementation of the sum() builtin function.

use std::mem;

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::{FrozenFunction, FrozenFunctions},
    heap::{DropGuard, HeapReadOutput},
    modules::ModuleFunctions,
    types::{BuiltinType, PyTrait, Type},
    value::Value,
};

/// Argument shape for `sum(iterable, /, start=0)` — Argument Clinic in
/// CPython, so `start` is keyword-capable while `iterable` is positional-only.
/// `at_most_total` reproduces `_PyArg_UnpackKeywords`' total pre-count
/// (`sum() takes at most 2 arguments (3 given)`).
#[derive(FromArgs)]
#[from_args(name = "sum", at_most_total)]
struct SumArgs {
    #[from_args(pos_only)]
    iterable: Value,
    #[from_args(default = Value::Int(0))]
    start: Value,
}

/// Implementation of the sum() builtin function.
///
/// Sums the items of an iterable from left to right with an optional start value.
/// The default start value is 0. Str and bytes start values are explicitly
/// rejected, pointing at `''.join(seq)` / `b''.join(seq)` instead.
pub fn builtin_sum(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let SumArgs { iterable, start } = SumArgs::from_args(args, vm)?;
    // Ranges yield only integers and these start values cannot call Python.
    // All other inputs use the same suspendable implementation.
    if matches!(iterable.read_heap(vm), Some(HeapReadOutput::Range(_)))
        && matches!(start, Value::Int(_) | Value::Float(_))
    {
        return sum_native(iterable, start, vm).map(CallResult::Value);
    }
    vm.call_frozen_exact(
        FrozenFunction::Sum,
        [
            iterable,
            start,
            Value::Builtin(Builtins::Type(BuiltinType::Iterator)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
            Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::ValidateSumStart)),
        ],
    )
}

/// Numeric range summation avoids per-item Python call overhead.
fn sum_native(iterable: Value, start: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    defer_drop_mut!(start, vm);
    let iterator = iterable.into_py_iter(vm)?.require_synchronous(vm)?;
    defer_drop!(iterator, vm);
    let start = validate_start(ArgValues::One(mem::replace(start, Value::None)), vm)?;
    let mut total = DropGuard::new(start, vm);
    let (accumulator, vm) = total.as_parts_mut();
    let mut iterator = iterator.read(vm);
    while let Some(item) = iterator.py_next(vm)? {
        defer_drop!(item, vm);
        let next = accumulator.py_add(item, vm)?;
        mem::replace(accumulator, next).drop_with(vm);
    }
    Ok(total.into_inner())
}

/// CPython validates the start only after acquiring the iterator.
pub(crate) fn validate_start(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let start = args.get_one_arg("validate_sum_start", vm.heap)?;
    defer_drop!(start, vm);
    match start.py_type(vm) {
        Type::Str => Err(ExcType::type_error_sum_start("strings", "''")),
        Type::Bytes => Err(ExcType::type_error_sum_start("bytes", "b''")),
        _ => Ok(start.clone_with_heap(vm)),
    }
}

/// Left-to-right addition without materializing the input or repeating effects.
pub(crate) const SUM_SOURCE: &str = r"
def __monty_sum(iterable, total, iter_, next_, validate):
    iterator = iter_(iterable)
    total = validate(total)
    sentinel = []
    while True:
        item = next_(iterator, sentinel)
        if item is sentinel:
            return total
        total = total + item
";
