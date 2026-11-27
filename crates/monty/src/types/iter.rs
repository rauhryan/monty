//! Shared helpers for Python's iteration protocol.
//!
//! Concrete iterator state lives beside its iterable type. This module only
//! provides iterator validation, suspendable collection and native-only drains.

use monty_types::{ResourceError, ResourceTracker};

use crate::{
    args::ArgValues,
    builtins::sorted,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    frozen::{FrozenFunction, FrozenFunctions},
    heap::{DropGuard, DropWithContext, HeapData, HeapReadOutput},
    modules::ModuleFunctions,
    resource_checks::check_estimated_size,
    types::{List, PyTrait, Tuple, Type, str},
    value::{VALUE_SIZE, Value, ValueRead},
};

/// Validates a user `__iter__` result after any suspension in the method.
pub(crate) const ITER_SOURCE: &str = r"
def __monty_iter(object, get_iter, validate):
    return validate(get_iter(object))
";

/// Advances an already acquired custom iterator through ordinary VM calls.
/// The loop body executes outside the exhaustion handler and no close is forwarded.
pub(crate) const FOR_ITERATOR_SOURCE: &str = r"
def __monty_for_iterator(iterator, next_, stop_iteration):
    while True:
        try:
            item = next_(iterator)
        except stop_iteration:
            return
        yield item
        item = None
";

/// Invokes the user protocol without re-entering the instance's validation frame.
pub(crate) fn call_get_iter(args: ArgValues, vm: &mut VM<'_>) -> RunResult<CallResult> {
    let object = args.get_one_arg("get_iter", vm.heap)?;
    defer_drop!(object, vm);
    let Some(HeapReadOutput::Instance(instance)) = object.read_heap(vm) else {
        unreachable!("only instances need frozen iterator validation")
    };
    instance.call_iter(vm)
}

/// Returns an owned iterator unchanged, rejecting invalid user `__iter__` results.
pub(crate) fn call_validate_iterator(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let iterator = args.get_one_arg("validate_iterator", vm.heap)?;
    if iterator.py_is_iterator(vm) {
        Ok(iterator)
    } else {
        let error = ExcType::type_error_iter_returned_non_iterator(&iterator.py_type_name(vm));
        iterator.drop_with(vm);
        Err(error)
    }
}

/// Largest iterator hint used to preallocate a native collection.
const MAX_PREALLOCATION_HINT: usize = 65_536;

/// Validates and clamps an iterator capacity hint before native allocation.
pub(crate) fn checked_preallocation_hint(
    hint: usize,
    elem_size: usize,
    tracker: &ResourceTracker,
) -> Result<usize, ResourceError> {
    let hint = hint.min(MAX_PREALLOCATION_HINT);
    check_estimated_size(hint.saturating_mul(elem_size), tracker)?;
    Ok(hint)
}

/// Adapts a retained Python iterator to Rust's `Iterator` with memory checks.
struct HeapedIterator<'this, 'h, I: CollectIter<'h>> {
    iter: &'this mut I,
    vm: &'this mut VM<'h>,
    error: &'this mut Option<RunError>,
    preallocation_hint: usize,
    yielded: usize,
}

impl<'h, I: CollectIter<'h>> Iterator for HeapedIterator<'_, 'h, I> {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next_value(self.vm) {
            Ok(None) => None,
            Err(error) => {
                *self.error = Some(error);
                None
            }
            Ok(Some(value)) => {
                self.yielded += 1;
                let estimated = self.yielded.saturating_mul(VALUE_SIZE);
                // Size alone does not bound a source whose items are cheap or
                // interned, and the drain reaches no VM dispatch checkpoint of
                // its own, so the time limits need their own poll here.
                let checked = check_estimated_size(estimated, &self.vm.heap.tracker)
                    .and_then(|()| self.vm.heap.tracker.check_time_every(self.yielded));
                match checked {
                    Ok(()) => Some(value),
                    Err(error) => {
                        *self.error = Some(error.into());
                        value.drop_with(self.vm);
                        None
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.iter.remaining(self.vm).min(self.preallocation_hint);
        (remaining, Some(remaining))
    }
}

/// Interface used to drain a retained Python iterator through Rust's `collect()`.
trait CollectIter<'h> {
    /// Advances the iterator by one item.
    fn next_value(&mut self, vm: &mut VM<'h>) -> RunResult<Option<Value>>;

    /// Returns an internal remaining-length hint when available.
    fn remaining(&self, vm: &VM<'h>) -> usize;
}

impl<'h> CollectIter<'h> for ValueRead<'h, '_> {
    fn next_value(&mut self, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        self.py_next(vm)
    }

    fn remaining(&self, vm: &VM<'h>) -> usize {
        self.iter_size_hint(vm)
    }
}

/// Collects every item yielded by an iterable into a `Vec`.
pub fn collect_iterable(value: &Value, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    let iterator = value.py_iter(vm)?.require_synchronous(vm)?;
    collect_python_iterator(iterator, vm)
}

/// Collects every item yielded by an owned iterable into a target collection.
pub(crate) fn collect_owned_iterable<'h, T>(value: Value, vm: &mut VM<'h>) -> RunResult<T>
where
    T: Default + Extend<Value> + DropWithContext<VM<'h>>,
{
    let iterator = value.into_py_iter(vm)?.require_synchronous(vm)?;
    collect_python_iterator(iterator, vm)
}

/// Drains an owned Python iterator with incremental allocation checks.
pub(crate) fn collect_python_iterator<'h, T>(iterator: Value, vm: &mut VM<'h>) -> RunResult<T>
where
    T: Default + Extend<Value> + DropWithContext<VM<'h>>,
{
    defer_drop!(iterator, vm);
    let mut iterator = iterator.read(vm);
    let preallocation_hint = checked_preallocation_hint(iterator.iter_size_hint(vm), VALUE_SIZE, &vm.heap.tracker)?;
    let mut values_guard = DropGuard::new(T::default(), vm);
    let (values, vm) = values_guard.as_parts_mut();
    let mut error = None;
    values.extend(HeapedIterator {
        iter: &mut iterator,
        vm,
        error: &mut error,
        preallocation_hint,
        yielded: 0,
    });
    if let Some(error) = error {
        Err(error)
    } else {
        Ok(values_guard.into_inner())
    }
}

/// Pulls at most `limit` items from an iterable, stopping early.
pub fn collect_iterable_bounded(value: &Value, limit: usize, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    let iterator = value.py_iter(vm)?.require_synchronous(vm)?;
    defer_drop!(iterator, vm);
    let mut iterator = iterator.read(vm);
    let mut items_guard = DropGuard::new(Vec::new(), vm);
    let (items, vm) = items_guard.as_parts_mut();
    while items.len() < limit {
        match iterator.py_next(vm)? {
            Some(value) => items.push(value),
            None => break,
        }
    }
    Ok(items_guard.into_inner())
}

/// The collection calling convention retained by snapshots written before format 48.
/// New execution uses COLLECT_SOURCE; restored callers keep their original arguments.
pub(crate) const COLLECT_GENERATOR_SOURCE: &str = r"
def __monty_collect_generator(generator, as_tuple, tuple_):
    result = []
    for item in generator:
        result.append(item)
    if as_tuple:
        return tuple_(result)
    return result
";

/// PEP 380 delegation for iterables whose iterator is not itself a generator.
/// Each protocol call uses an ordinary frame and retains its exception identity.
pub(crate) const DELEGATE_ITERABLE_SOURCE: &str = r"
def __monty_delegate(iterable, iter_, next_, getattr_, stop_, exit_, base_, attribute_):
    iterator = iter_(iterable)
    try:
        value = next_(iterator)
    except stop_ as error:
        return error.value
    while True:
        try:
            sent = yield value
        except exit_:
            try:
                close = getattr_(iterator, 'close')
            except attribute_:
                pass
            else:
                close()
            raise
        except base_ as error:
            try:
                throw = getattr_(iterator, 'throw')
            except attribute_:
                raise error
            try:
                value = throw(error)
            except stop_ as done:
                return done.value
        else:
            try:
                if sent is None:
                    value = next_(iterator)
                else:
                    value = iterator.send(sent)
            except stop_ as done:
                return done.value
";

/// Native iterator consumers have a synchronous activation boundary. The ordinary
/// frame drives generator state without retaining a Rust stack across host calls;
/// such calls are rejected by the existing synchronous-boundary policy.
pub(crate) const GENERATOR_NEXT_SOURCE: &str = r"
def __monty_generator_next(generator, next_):
    return next_(generator)
";

/// Native operation applied after the VM has finished collecting an iterable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum CollectionResult {
    List,
    Tuple,
    Sorted,
    Join,
    /// Validate before replay so a collected iterator does not acquire list error wording.
    Unpack,
}

/// Exact sequences can be copied without running iterator methods or host calls.
pub(crate) fn is_materialized(value: &Value, vm: &VM<'_>) -> bool {
    matches!(value.py_type(vm), Type::List | Type::Tuple)
}

/// Collects on normal VM frames, then applies a native operation to the resulting list.
/// Exact sequences and integer ranges cannot call user code; native collection
/// also preserves their exact capacity hints instead of growing a list incrementally.
pub(crate) fn collect_then(
    iterable: Value,
    result: CollectionResult,
    context: Value,
    limit: Option<usize>,
    vm: &mut VM<'_>,
) -> RunResult<CallResult> {
    if is_materialized(&iterable, vm) || iterable.py_type(vm) == Type::Range {
        let mut owned = DropGuard::new((iterable, context), vm);
        let ((iterable, _), vm) = owned.as_parts();
        let items = match limit {
            Some(limit) => collect_iterable_bounded(iterable, limit, vm)?,
            None => collect_iterable(iterable, vm)?,
        };
        let ((iterable, context), vm) = owned.into_parts();
        iterable.drop_with(vm);
        let items = Value::Ref(vm.heap.allocate(HeapData::List(List::new(items))));
        return finish_collection(result, ArgValues::Two(items, context), vm).map(CallResult::Value);
    }
    vm.call_frozen_exact(
        FrozenFunction::Collect,
        [
            iterable,
            Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::FinishCollection(result))),
            context,
            limit.map_or(Value::None, |limit| {
                Value::Int(i64::try_from(limit).expect("collection limit fits i64"))
            }),
        ],
    )
}

/// One collection loop serves syntax, constructors, sorting and joining.
/// A bounded request stops before pulling another item once the limit is reached.
pub(crate) const COLLECT_SOURCE: &str = r"
def __monty_collect(iterable, finish, context, remaining):
    items = []
    if remaining != 0:
        for item in iterable:
            items.append(item)
            if remaining is not None:
                remaining -= 1
                if remaining == 0:
                    break
    return finish(items, context)
";

/// Consumes the collected list and the continuation's owned context.
pub(crate) fn finish_collection(result: CollectionResult, args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let (items, context) = args.get_two_args("finish_collection", vm.heap)?;
    match result {
        CollectionResult::List => {
            context.drop_with(vm);
            Ok(items)
        }
        CollectionResult::Tuple => {
            context.drop_with(vm);
            Tuple::init(vm, ArgValues::One(items))
        }
        CollectionResult::Sorted => sorted::finish_sorted(items, context, vm),
        CollectionResult::Join => str::finish_join(items, context, vm),
        CollectionResult::Unpack => vm.validate_collected_unpack(items, context),
    }
}
