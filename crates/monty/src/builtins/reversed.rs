//! Implementation of the reversed() builtin function.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::HeapData,
    types::{
        deque::DequeIterator,
        dict::{DictItemIterator, DictKeyIterator, DictValueIterator},
        range::RangeIterator,
    },
    value::Value,
};

/// Argument shape for `reversed(sequence, /)` — positional-only, so
/// `style = unpack` gives CPython's exact-arity wording
/// (`reversed expected 1 argument, got 2`) and the blanket
/// `reversed() takes no keyword arguments` rejection.
#[derive(FromArgs)]
#[from_args(name = "reversed", style = unpack)]
struct ReversedArgs {
    #[from_args(pos_only)]
    sequence: Value,
}

/// Reverse supported built-in containers without collecting their contents.
pub fn builtin_reversed(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ReversedArgs { sequence } = ReversedArgs::from_args(args, vm)?;
    defer_drop!(sequence, vm);
    if !is_reversible(sequence, vm) {
        return Err(ExcType::type_error_not_reversible(&sequence.py_type_name(vm)));
    }
    if let Value::Ref(id) = sequence {
        if let HeapData::Range(range) = vm.heap.get(*id) {
            let range = *range;
            return Ok(CallResult::Value(RangeIterator::allocate_reversed(range, vm)));
        }
        let special = match vm.heap.get(*id) {
            HeapData::Dict(_) => Some((*id, 0)),
            HeapData::DictKeysView(view) => Some((view.dict_id(), 0)),
            HeapData::DictItemsView(view) => Some((view.dict_id(), 1)),
            HeapData::DictValuesView(view) => Some((view.dict_id(), 2)),
            HeapData::Deque(_) => Some((*id, 3)),
            _ => None,
        };
        if let Some((id, kind)) = special {
            let iterator = match kind {
                0 => DictKeyIterator::allocate_reversed(id, vm),
                1 => DictItemIterator::allocate_reversed(id, vm),
                2 => DictValueIterator::allocate_reversed(id, vm),
                _ => {
                    let iterator = DequeIterator::reversed(id, vm);
                    let result = Value::Ref(vm.heap.allocate(HeapData::DequeIterator(iterator)));
                    vm.heap.inc_ref(id);
                    result
                }
            };
            return Ok(CallResult::Value(iterator));
        }
    }
    vm.call_frozen_exact(
        FrozenFunction::Reversed,
        [
            sequence.clone_with_heap(vm.heap),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Len)),
            Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
            Value::Builtin(Builtins::ExcType(ExcType::IndexError)),
        ],
    )
}

/// A fixed starting index observes list replacement/shrinkage without seeing later appends.
pub(crate) const REVERSED_SOURCE: &str = r"
def __monty_reversed(sequence, len_, stop_, index_error):
    class reversed:
        def __init__(self, sequence, index):
            self.sequence = sequence
            self.index = index

        def __iter__(self):
            return self

        def __next__(self):
            if self.index >= 0:
                try:
                    item = self.sequence[self.index]
                except index_error:
                    pass
                else:
                    self.index -= 1
                    return item
            self.sequence = None
            self.index = -1
            raise stop_

    return reversed(sequence, len_(sequence) - 1)
";

/// Whether `reversed()` accepts this value, mirroring CPython's requirement of
/// `__reversed__` or `__len__` + `__getitem__`.
///
/// Deliberately a positive allowlist: a newly iterable type must opt in here
/// rather than becoming reversible by accident. Sets and iterators are excluded
/// (no ordering / one-shot), as are user instances — `__reversed__` is not
/// dispatched, see `limitations/classes.md`.
fn is_reversible(value: &Value, vm: &VM<'_>) -> bool {
    match value {
        Value::InternString(_) | Value::InlineString(_) | Value::InternBytes(_) => true,
        Value::Ref(id) => matches!(
            vm.heap.get(*id),
            HeapData::List(_)
                | HeapData::Tuple(_)
                | HeapData::NamedTuple(_)
                | HeapData::Deque(_)
                | HeapData::Str(_)
                | HeapData::Bytes(_)
                | HeapData::Range(_)
                | HeapData::Dict(_)
                | HeapData::DictKeysView(_)
                | HeapData::DictItemsView(_)
                | HeapData::DictValuesView(_)
        ),
        _ => false,
    }
}
