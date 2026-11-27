//! Sorting after collection through the VM's iteration protocol.

use crate::{
    args::{ArgValues, FromArgs, KwargsValues},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData, HeapReadOutput},
    intern::StaticStrings,
    types::{
        Dict,
        iter::{CollectionResult, collect_then},
    },
    value::Value,
};

/// Sorting validates keywords after consuming its input, as list.sort does.
pub fn builtin_sorted(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let SortedArgs { mut positional, kwargs } = SortedArgs::from_args(args, vm)?;
    if positional.len() != 1 {
        let count = positional.len();
        positional.drop_with(vm);
        kwargs.drop_with(vm);
        return Err(ExcType::type_error_expected_exact("sorted", 1, count));
    }
    let iterable = positional.pop().expect("one argument checked");
    defer_drop!(iterable, vm);
    let kwargs = Dict::from_pairs(kwargs.into_iter().collect(), vm)?;
    let context = Value::Ref(vm.heap.allocate(HeapData::Dict(kwargs)));
    let iterable = iterable.clone_with_heap(vm);
    collect_then(iterable, CollectionResult::Sorted, context, None, vm)
}

/// Preserve keywords for list.sort, which validates them after consuming the input.
#[derive(FromArgs)]
#[from_args(name = "sorted")]
struct SortedArgs {
    #[from_args(varargs)]
    positional: Vec<Value>,
    #[from_args(varkwargs)]
    kwargs: KwargsValues,
}

/// Sorts the newly collected list in place; no second collection is needed.
pub(crate) fn finish_sorted(items: Value, kwargs: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    defer_drop!(items, vm);
    defer_drop!(kwargs, vm);
    let Some(HeapReadOutput::Dict(dict)) = kwargs.read_heap(vm) else {
        unreachable!("sorted context is a keyword dictionary")
    };
    let pairs = dict.clone_all_pairs(vm)?;
    let args = ArgValues::Kwargs(KwargsValues::Pairs(pairs));
    vm.call_attr(
        items.clone_with_heap(vm.heap),
        vm.interns.intern_static(StaticStrings::Sort),
        args,
    )?
    .require_synchronous(vm)?
    .drop_with(vm);
    Ok(items.clone_with_heap(vm))
}
