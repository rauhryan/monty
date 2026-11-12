//! Lazy enumerate state on ordinary, suspendable iterator-method frames.

use crate::{
    args::ArgValues,
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::DropWithContext,
    types::BuiltinType,
    value::Value,
};

/// Validate the index before acquiring the iterator, without consuming any items.
pub fn builtin_enumerate(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let (iterable, start) = extract_enumerate_args(args, vm)?;
    defer_drop!(iterable, vm);
    defer_drop!(start, vm);
    let index = match start {
        Some(value) => value.py_index(vm)?,
        None => Value::Int(0),
    };
    vm.call_frozen_exact(
        FrozenFunction::Enumerate,
        [
            iterable.clone_with_heap(vm.heap),
            index,
            Value::Builtin(Builtins::Type(BuiltinType::Iterator)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
        ],
    )
}

/// A method frame can fail without closing the iterator, unlike a generator body.
/// Only the source iterator and next index remain owned between successful calls.
pub(crate) const ENUMERATE_SOURCE: &str = r"
def __monty_enumerate(iterable, index, iter_, next_):
    class enumerate:
        def __init__(self, iterator, index):
            self.iterator = iterator
            self.index = index

        def __iter__(self):
            return self

        def __next__(self):
            item = next_(self.iterator)
            index = self.index
            self.index = index + 1
            return (index, item)

    return enumerate(iter_(iterable), index)
";

/// Extracts `enumerate(iterable, start=0)` arguments, faithfully mirroring
/// CPython's hand-written `enumerate_vectorcall` (Objects/enumobject.c) —
/// which `#[derive(FromArgs)]` cannot express. Its quirks are deliberate:
/// keyword names are validated by *position* against the accepted shapes, and
/// zero positionals with an unrecognised keyword shape reports the missing
/// `iterable` even when `iterable=` was actually passed.
fn extract_enumerate_args(args: ArgValues, vm: &mut VM<'_>) -> RunResult<(Value, Option<Value>)> {
    let (mut pos, kwargs) = args.into_parts();
    let n_pos = pos.len();
    let total = n_pos + kwargs.len();
    let mut kw = kwargs.into_iter();
    // Pull up to two of each stream; each accepted call shape is one pattern
    // arm, and the catch-all owns the leftovers on the arity-error path.
    match (pos.next(), pos.next(), kw.next(), kw.next()) {
        // enumerate(x) / enumerate(x, s)
        (Some(iterable), start, None, None) if total <= 2 => Ok((iterable, start)),
        // enumerate(x, start=s)
        (Some(iterable), None, Some(kv), None) => match take_kwarg(kv, "start", vm) {
            Ok(start) => Ok((iterable, Some(start))),
            Err(err) => {
                iterable.drop_with(vm);
                Err(err)
            }
        },
        // enumerate(iterable=x)
        (None, None, Some(kv), None) => Ok((take_kwarg(kv, "iterable", vm)?, None)),
        // enumerate(iterable=x, start=s) — either keyword order
        (None, None, Some(kv0), Some(kv1)) if total == 2 => two_kwarg_form(kv0, kv1, vm),
        // Anything else is an arity error.
        (p0, p1, k0, k1) => {
            ((p0, p1), (k0, k1)).drop_with(vm);
            pos.drop_with(vm);
            kw.drop_with(vm);
            if n_pos == 0 {
                // CPython reports the missing `iterable` for every
                // zero-positional shape it doesn't recognise — even
                // `enumerate(start=1, x=1, iterable=[1])`.
                Err(ExcType::type_error_missing_required_no_pos("enumerate", "iterable"))
            } else {
                // `n_pos > 0` in this arm, so never the all-keyword wording.
                Err(ExcType::type_error_method_at_most("enumerate", 2, total, false))
            }
        }
    }
}

/// The `enumerate(iterable=..., start=...)` form: CPython accepts both
/// keyword orders, checking `start` first and reporting the first
/// out-of-place name.
fn two_kwarg_form(kv0: (Value, Value), kv1: (Value, Value), vm: &mut VM<'_>) -> RunResult<(Value, Option<Value>)> {
    // Decide the shape by reference before any ownership moves.
    let swapped = key_check(&kv0.0, "start", vm).is_ok();
    let checked = if swapped {
        key_check(&kv1.0, "iterable", vm)
    } else {
        key_check(&kv0.0, "iterable", vm).and(key_check(&kv1.0, "start", vm))
    };
    match checked {
        Ok(()) => {
            let ((key0, val0), (key1, val1)) = (kv0, kv1);
            (key0, key1).drop_with(vm);
            Ok(if swapped {
                (val1, Some(val0))
            } else {
                (val0, Some(val1))
            })
        }
        Err(err) => {
            (kv0, kv1).drop_with(vm);
            Err(err)
        }
    }
}

/// Consumes a kwarg pair, returning its value if the key is `expected`.
fn take_kwarg(kv: (Value, Value), expected: &str, vm: &mut VM<'_>) -> RunResult<Value> {
    match key_check(&kv.0, expected, vm) {
        Ok(()) => {
            let (key, value) = kv;
            key.drop_with(vm);
            Ok(value)
        }
        Err(err) => {
            kv.drop_with(vm);
            Err(err)
        }
    }
}

/// Errors unless `key` is the string `expected`, mirroring CPython's
/// `check_keyword`: a non-string key raises `keywords must be strings`, any
/// other name the `invalid keyword argument` wording.
fn key_check(key: &Value, expected: &str, vm: &VM<'_>) -> RunResult<()> {
    let Some(key) = key.as_either_str(vm.heap) else {
        return Err(ExcType::type_error_kwargs_nonstring_key());
    };
    let key = key.as_str(vm.interns);
    if key == expected {
        Ok(())
    } else {
        Err(ExcType::type_error_invalid_keyword_argument("enumerate", key))
    }
}
