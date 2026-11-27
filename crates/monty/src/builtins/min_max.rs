//! Implementation of the min() and max() builtin functions.

use std::cmp::Ordering;

use crate::{
    args::{ArgValues, FromArgs},
    builtins::Builtins,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    frozen::FrozenFunction,
    heap::DropGuard,
    types::{CmpOrder, PyTrait, allocate_tuple, str::allocate_string},
    value::Value,
};

/// Implementation of the min() builtin function.
///
/// Returns the smallest item in an iterable or the smallest of two or more arguments.
/// Supports two forms:
/// - `min(iterable)` - returns smallest item from iterable
/// - `min(arg1, arg2, ...)` - returns smallest of the arguments
pub fn builtin_min(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let MinArgs { args, key, default } = MinArgs::from_args(args, vm)?;
    dispatch_min_max(vm, args, key, default, true)
}

/// Implementation of the max() builtin function.
///
/// Returns the largest item in an iterable or the largest of two or more arguments.
/// Supports two forms:
/// - `max(iterable)` - returns largest item from iterable
/// - `max(arg1, arg2, ...)` - returns largest of the arguments
pub fn builtin_max(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let MaxArgs { args, key, default } = MaxArgs::from_args(args, vm)?;
    dispatch_min_max(vm, args, key, default, false)
}

/// Both iterable and positional forms use the same loop and suspendable key calls.
fn dispatch_min_max(
    vm: &mut VM<'_>,
    args: Vec<Value>,
    key: Value,
    default: Option<Value>,
    is_min: bool,
) -> RunResult<CallResult> {
    let name = if is_min { "min" } else { "max" };
    let mut arguments = DropGuard::new((args, (key, default)), vm);
    let ((args, (_, default)), _) = arguments.as_parts_mut();
    if args.is_empty() {
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            format!("{name} expected at least 1 argument, got 0"),
        )
        .into());
    }
    if args.len() > 1 && default.is_some() {
        return Err(default_with_multiple_args(name));
    }
    let ((mut args, (key, default)), vm) = arguments.into_parts();
    let iterable = if args.len() == 1 {
        args.pop().expect("one argument checked")
    } else {
        allocate_tuple(args.into(), vm.heap)
    };
    let supplied = Value::Bool(default.is_some());
    let message = allocate_string(format!("{name}() iterable argument is empty"), vm.heap);
    vm.call_frozen_exact(
        FrozenFunction::MinMax,
        [
            iterable,
            key,
            default.unwrap_or(Value::None),
            supplied,
            Value::Bool(is_min),
            Value::Builtin(Builtins::ExcType(ExcType::ValueError)),
            message,
        ],
    )
}

/// Retain only the current winner; ties preserve the first item.
pub(crate) const MIN_MAX_SOURCE: &str = r"
def __monty_min_max(iterable, key, default, supplied, is_min, error, message):
    found = False
    result = None
    result_key = None
    for item in iterable:
        item_key = item if key is None else key(item)
        if not found or (item_key < result_key if is_min else item_key > result_key):
            result = item
            result_key = item_key
            found = True
    if found:
        return result
    if supplied:
        return default
    raise error(message)
";

/// Argument shape for `min(*args, key=None, default=...)`.
///
/// `key` is held as `Value` so `key=None` can be normalised to "no key
/// function". `default` is `Option<Value>` (with `default` attr) so the
/// implementation can distinguish "not provided" from "provided with any
/// value" — that distinction matters because `default=` is only valid with a
/// single iterable argument.
#[derive(FromArgs)]
#[from_args(name = "min")]
struct MinArgs {
    #[from_args(varargs)]
    args: Vec<Value>,
    #[from_args(default = Value::None)]
    key: Value,
    #[from_args(default)]
    default: Option<Value>,
}

/// Argument shape for `max(*args, key=None, default=...)`.
///
/// See [`MinArgs`] for field semantics; the only difference is the function
/// name used in error messages.
#[derive(FromArgs)]
#[from_args(name = "max")]
struct MaxArgs {
    #[from_args(varargs)]
    args: Vec<Value>,
    #[from_args(default = Value::None)]
    key: Value,
    #[from_args(default)]
    default: Option<Value>,
}

/// Returns whether `candidate` should replace `current` as the best value seen so far.
///
/// `min()` replaces the current winner when the new candidate compares smaller,
/// while `max()` replaces it when the new candidate compares larger. Equal values
/// keep the existing winner so ties preserve the first-seen item, matching CPython.
pub(crate) fn candidate_wins(current: &Value, candidate: &Value, is_min: bool, vm: &mut VM<'_>) -> RunResult<bool> {
    let ordering = match candidate.py_cmp(current, vm)? {
        CmpOrder::Ordered(ordering) => ordering,
        // A `NaN` candidate (or `NaN`-carrying container) is neither smaller nor
        // larger, so it never displaces the incumbent — matching CPython, where
        // `min`/`max` only swap on a strict `<`/`>` and `NaN` yields neither.
        CmpOrder::Unordered => return Ok(false),
        CmpOrder::Incomparable => return Err(ord_not_supported(candidate, current, is_min, vm)),
    };

    Ok((is_min && ordering == Ordering::Less) || (!is_min && ordering == Ordering::Greater))
}

/// Creates the CPython-compatible error for `default=` with multiple positional args.
#[cold]
fn default_with_multiple_args(func_name: &str) -> RunError {
    SimpleException::new_msg(
        ExcType::TypeError,
        format!("Cannot specify a default for {func_name}() with multiple positional arguments"),
    )
    .into()
}

#[cold]
fn ord_not_supported(left: &Value, right: &Value, is_min: bool, vm: &VM<'_>) -> RunError {
    let left_type = left.py_type_name(vm);
    let right_type = right.py_type_name(vm);
    let operator = if is_min { "<" } else { ">" };
    ExcType::type_error_ordering(operator, &left_type, &right_type)
}
