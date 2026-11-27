//! Implementation of the sorted() builtin function.

use std::mem;

use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::{DropGuard, DropWithContext, HeapData, HeapReadOutput},
    sorting::parse_and_sort,
    types::{Dict, List, iter::collect_owned_iterable},
    value::Value,
};

/// Implementation of the sorted() builtin function.
///
/// Returns a new sorted list from the items in an iterable. CPython's
/// `sorted(iterable, /, *, key=None, reverse=False)` is implemented by
/// converting the iterable to a list and delegating to `list.sort`, so
/// the kwargs and any kwarg-related errors are owned by `sort`. We mirror
/// that by extracting the iterable positionally and handing the rest off
/// to [`parse_and_sort`] — the same entry point `list.sort` uses — so
/// unknown-kwarg errors uniformly read `sort() got an unexpected keyword
/// argument 'X'` without any wording overrides.
pub fn builtin_sorted(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let (mut pos_iter, kwargs) = args.into_parts();
    defer_drop_mut!(kwargs, vm);
    let pos_count = pos_iter.len();
    if pos_count != 1 {
        pos_iter.drop_with(vm);
        return Err(ExcType::type_error_expected_exact("sorted", 1, pos_count));
    }
    let iterable = pos_iter.next().expect("checked pos_count == 1");

    if matches!(iterable.read_heap(vm), Some(HeapReadOutput::Generator(_))) {
        defer_drop!(iterable, vm);
        let kwargs = Dict::from_pairs(mem::replace(kwargs, KwargsValues::Empty).into_iter().collect(), vm)?;
        let kwargs = Value::Ref(vm.heap.allocate(HeapData::Dict(kwargs)));
        let iterable = iterable.clone_with_heap(vm);
        return vm.call_frozen_exact(FrozenFunction::Sorted, [iterable, kwargs]);
    }
    let items: Vec<_> = collect_owned_iterable(iterable, vm)?;
    let mut items_guard = DropGuard::new(items, vm);
    let (items, vm) = items_guard.as_parts_mut();

    let sort_args = if kwargs.is_empty() {
        ArgValues::Empty
    } else {
        ArgValues::Kwargs(mem::replace(kwargs, KwargsValues::Empty))
    };
    parse_and_sort(items, sort_args, vm)?;

    let (items, vm) = items_guard.into_parts();
    let heap_id = vm.heap.allocate(HeapData::List(List::new(items)));
    Ok(CallResult::Value(Value::Ref(heap_id)))
}

/// Sorting materializes its input before evaluating keys.
pub(crate) const SORTED_SOURCE: &str = r"
def __monty_sorted(iterable, kwargs):
    items = []
    for item in iterable:
        items.append(item)
    items.sort(**kwargs)
    return items
";
