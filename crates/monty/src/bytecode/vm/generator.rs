//! Synchronous generator activations. No Rust run-loop re-entry is retained.
//!
//! The executing frame owns one pin; dormant state owns the locals, operands,
//! namespace and caught exceptions. Suspension moves these owners, never clones
//! them. Physical disposal releases owners without dispatching guest cleanup.

use std::{collections::HashSet, mem};

use super::{CallFrame, CallResult, CaughtException, SerializedFrame, VM, stack_index};
use crate::{
    args::{ArgValues, FromArgs},
    builtins::Builtins,
    exception_private::{ExcType, ExcTypeExt, ExceptionObject, RunError, RunResult, SimpleException},
    heap::{
        ContainsHeap, DropGuard, DropWithContext, HeapData, HeapId, HeapItem, HeapObjectRead, HeapReadOutput,
        HeapReader,
    },
    intern::FunctionId,
    types::{PyTrait, Type, allocate_tuple},
    value::{EitherStr, Value},
};

/// A boxed payload to keep the native/Wasm heap slot ceilings unchanged.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Generator {
    pub(crate) function: FunctionId,
    state: GeneratorState,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum GeneratorState {
    New { slots: Vec<Value>, globals: Option<HeapId> },
    Running,
    Suspended { segment: SavedGenerator },
    Closed,
}

/// One dormant generator frame. Ordinary calls have returned before Yield;
/// active host cuts retain all frames in the VM's normal snapshot instead.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SavedGenerator {
    stack: Vec<Value>,
    frame: SerializedFrame,
    exceptions: Vec<CaughtException>,
    fault_ip: usize,
    #[serde(default)]
    pub(super) delegating: bool,
}

/// Persisted return route on the generator's live frame, including its sole pin.
/// Task parking and host snapshots transfer this together with the frame.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GeneratorActivation {
    generator: Value,
    /// Opt-in native entry/delegate: Yield awaits on the existing Task/Scheduler.
    #[serde(default)]
    pub(super) native: bool,
    default: Option<Value>,
    #[serde(default)]
    delegated: bool,
    #[serde(default)]
    pub(super) delegating: bool,
}

impl<C: ContainsHeap> DropWithContext<C> for GeneratorActivation {
    fn drop_with(self, ctx: &mut C) {
        // Running has no dormant roots. Closing never executes finally/close.
        if let Value::Ref(id) = &self.generator {
            HeapReader::with(ctx.heap_mut(), &mut (), |heap, ()| {
                let HeapReadOutput::Generator(generator) = heap.read(*id) else {
                    unreachable!("activation pin must name a generator")
                };
                assert!(matches!(generator.get(heap).state, GeneratorState::Running));
                generator.get_mut(heap).state = GeneratorState::Closed;
            });
        }
        self.generator.drop_with(ctx);
        self.default.drop_with(ctx);
    }
}

impl Generator {
    pub(crate) fn new(function: FunctionId, slots: Vec<Value>, globals: Option<HeapId>) -> Self {
        Self {
            function,
            state: GeneratorState::New { slots, globals },
        }
    }

    /// Every owned edge, with multiplicity. Frame indices and function IDs are
    /// borrowed metadata. Frozen catches retain both their Value and error owner.
    pub(crate) fn for_each_child_id(&self, mut visit: impl FnMut(HeapId)) {
        let (slots, globals, saved) = match &self.state {
            GeneratorState::New { slots, globals } => (slots, *globals, None),
            GeneratorState::Suspended { segment } => (&segment.stack, None, Some(segment)),
            GeneratorState::Running | GeneratorState::Closed => return,
        };
        for value in slots {
            if let Value::Ref(id) = value {
                visit(*id);
            }
        }
        if let Some(id) = globals {
            visit(id);
        }
        if let Some(saved) = saved {
            saved.frame.for_each_child_id(&mut visit);
            for caught in &saved.exceptions {
                caught.for_each_owned_id(&mut visit);
            }
        }
    }
}

impl HeapItem for Generator {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Trace and destruction enumerate exactly the same edges.
        self.for_each_child_id(|id| stack.push(id));
        // Values have debug ownership sentinels; the IDs above are now owned by
        // the heap's iterative destruction queue, rather than these wrappers.
        #[cfg(feature = "memory-model-checks")]
        match &mut self.state {
            GeneratorState::New { slots, .. } => {
                for value in slots {
                    value.dec_ref_forget();
                }
            }
            GeneratorState::Suspended { segment } => {
                for value in &mut segment.stack {
                    value.dec_ref_forget();
                }
                for caught in &mut segment.exceptions {
                    caught.forget_owned_value();
                }
            }
            GeneratorState::Running | GeneratorState::Closed => {}
        }
    }
}

#[derive(FromArgs)]
#[from_args(name = "send")]
struct SendArgs {
    #[from_args(pos_only)]
    value: Value,
}

/// The one-argument throw form; normalization follows ordinary exception calls.
#[derive(FromArgs)]
#[from_args(name = "throw")]
struct ThrowArgs {
    #[from_args(pos_only)]
    exception: Value,
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, Generator> {
    fn py_type(&self, _: &VM<'h>) -> Type {
        Type::WalkGenerator
    }
    fn py_len(&self, _: &VM<'h>) -> Option<usize> {
        None
    }
    fn py_eq_impl(&self, _: &Value, _: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }
    fn py_is_iterator(&self, _: &VM<'h>) -> bool {
        true
    }
    fn py_is_iterable(&self, _: &VM<'h>) -> bool {
        true
    }
    fn py_iter(&self, vm: &mut VM<'h>) -> RunResult<CallResult> {
        Ok(CallResult::Value(self.clone_value(vm.heap)))
    }
    fn py_next(&mut self, _: &mut VM<'h>) -> RunResult<Option<Value>> {
        Err(ExcType::not_implemented("generator iteration requires an outer VM activation").into())
    }
    fn py_call_attr(&mut self, vm: &mut VM<'h>, name: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        if name.as_str(vm.interns) == "_native1" {
            args.check_zero_args("_native1", vm.heap)?;
            Ok(CallResult::NativeGenerator(self.clone_value(vm.heap)))
        } else if matches!(name.as_str(vm.interns), "send" | "throw") {
            let throwing = name.as_str(vm.interns) == "throw";
            let mut value = if throwing {
                ThrowArgs::from_args(args, vm)?.exception
            } else {
                SendArgs::from_args(args, vm)?.value
            };
            if throwing {
                match &value {
                    Value::Builtin(Builtins::ExcType(exc)) => value = exc.call(vm, ArgValues::Empty)?,
                    Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Exception(_)) => {}
                    _ => {
                        value.drop_with(vm);
                        return Err(ExcType::type_error(
                            "exceptions must be classes or instances deriving from BaseException",
                        ));
                    }
                }
            }
            Ok(CallResult::ResumeGenerator {
                generator: self.clone_value(vm.heap),
                input: value,
                throwing,
                default: None,
            })
        } else {
            args.drop_with(vm);
            Err(ExcType::attribute_error("generator", name.as_str(vm.interns)))
        }
    }
}

impl VM<'_> {
    /// Reserve the dormant delegate prefix before moving any state. Root and
    /// resource rejection leave it retryable; a Running descendant fails at
    /// its parent's saved yield-from site after those ancestors are installed.
    pub(super) fn activate_generator(
        &mut self,
        generator: Value,
        input: Value,
        default: Option<Value>,
        throwing: bool,
    ) -> RunResult<()> {
        self.activate_generator_route(generator, input, default, throwing, false, false)
    }

    /// Explicit experiment entry; ordinary send/throw/await retain their semantics.
    pub(super) fn activate_native_generator(&mut self, generator: Value) -> RunResult<()> {
        self.activate_generator_route(generator, Value::None, None, false, false, true)
    }

    fn activate_generator_route(
        &mut self,
        generator: Value,
        input: Value,
        default: Option<Value>,
        throwing: bool,
        delegated: bool,
        native: bool,
    ) -> RunResult<()> {
        let mut request = DropGuard::new((generator, (input, default)), self);
        let ((generator, (input, _)), vm) = request.as_parts_mut();
        let mut cursor = &*generator;
        let mut frames = 0;
        let mut seen = HashSet::new();
        loop {
            let Value::Ref(id) = cursor else {
                unreachable!("generator request")
            };
            if !seen.insert(*id) {
                // This ancestor will be Running once installed. Forwarding to
                // it fails at the immediate parent's saved yield-from site.
                break;
            }
            let HeapData::Generator(object) = vm.heap.get(*id) else {
                unreachable!("generator request")
            };
            if matches!(&object.state, GeneratorState::Suspended { .. })
                || matches!(&object.state, GeneratorState::New { .. }) && !throwing
            {
                vm.admit_native_code(&vm.interns.get_function(object.function).code)?;
            }
            match &object.state {
                GeneratorState::Running if frames == 0 => {
                    return Err(ExcType::value_error("generator already executing"));
                }
                GeneratorState::Running => break,
                GeneratorState::New { .. } if !throwing && !matches!(input, Value::None) => {
                    return Err(ExcType::type_error(
                        "can't send non-None value to a just-started generator",
                    ));
                }
                GeneratorState::Closed => break,
                GeneratorState::New { .. } if throwing => break,
                GeneratorState::Suspended { segment } if segment.delegating => {
                    frames += 1;
                    cursor = segment.stack.last().expect("owned delegate slot");
                }
                _ => {
                    frames += 1;
                    break;
                }
            }
        }
        for reserved in 0..frames {
            if let Err(error) = vm.incr_recursion() {
                for _ in 0..reserved {
                    vm.decr_recursion();
                }
                return Err(error.into());
            }
        }
        let ((generator, (input, default)), vm) = request.into_parts();
        vm.activate_generator_admitted(generator, input, default, throwing, delegated, native)
    }

    /// Install the already-reserved frames without Rust run-loop reentry.
    fn activate_generator_admitted(
        &mut self,
        mut generator: Value,
        input: Value,
        mut default: Option<Value>,
        throwing: bool,
        mut delegated: bool,
        native: bool,
    ) -> RunResult<()> {
        loop {
            let Value::Ref(id) = &generator else {
                unreachable!("generator request")
            };
            let HeapReadOutput::Generator(object) = self.heap.read(*id) else {
                unreachable!("generator request")
            };
            let function = object.get(self.heap).function;
            if matches!(object.get(self.heap).state, GeneratorState::Running) {
                // Only a descendant can reach this after admission. The live
                // parent now owns its saved fault IP, handler state and pin.
                drop(object);
                generator.drop_with(self);
                input.drop_with(self);
                default.drop_with(self);
                return match self.handle_exception(ExcType::value_error("generator already executing")) {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }
            if matches!(object.get(self.heap).state, GeneratorState::Closed)
                || throwing && matches!(object.get(self.heap).state, GeneratorState::New { .. })
            {
                // Throw into New never enters the body, including its finally.
                let state = mem::replace(&mut object.get_mut(self.heap).state, GeneratorState::Closed);
                drop(object);
                if let GeneratorState::New { slots, globals } = state {
                    slots.drop_with(self);
                    globals.map(Value::Ref).drop_with(self);
                }
                generator.drop_with(self);
                if throwing {
                    default.drop_with(self);
                    if delegated && matches!(input.py_type(self), Type::Exception(ExcType::StopIteration)) {
                        // A New/Closed delegate's throw raises StopIteration as
                        // protocol completion; no running body raised this error.
                        let Some(HeapReadOutput::Exception(exception)) = input.read_heap(self) else {
                            unreachable!("normalized exception")
                        };
                        let value = exception.stop_value(self);
                        drop(exception);
                        input.drop_with(self);
                        self.complete_delegation(value);
                        return Ok(());
                    }
                    let error = self.make_exception(&input, false);
                    return match self.handle_exception_with_value(error, Some(input)) {
                        Some(error) => Err(error),
                        None => Ok(()),
                    };
                }
                input.drop_with(self);
                if native && !delegated {
                    self.push(Value::None);
                    return Ok(());
                }
                if delegated {
                    self.complete_delegation(Value::None);
                    return Ok(());
                }
                if let Some(value) = default {
                    self.push(value);
                    return Ok(());
                }
                return Err(self.generator_completion_error(Value::None));
            }
            let locals_count = u16::try_from(self.interns.get_function(function).namespace_size)
                .expect("function namespace size exceeds u16");
            let caller_offset = self.current_offset().expect("generator caller offset");
            let state = mem::replace(&mut object.get_mut(self.heap).state, GeneratorState::Running);
            drop(object);
            let stack_base = self.stack.len();
            let exception_base = self.exception_stack.len();
            let func = self.interns.get_function(function);
            let (mut frame, delegating, fault_ip) = match state {
                GeneratorState::New { slots, globals } => {
                    self.stack.extend(slots);
                    let namespace = globals.map(|globals| Box::new(super::FrameNamespace::Function { globals }));
                    (
                        CallFrame::new_function(
                            &func.code,
                            stack_base,
                            locals_count,
                            exception_base,
                            function,
                            Some(caller_offset),
                            namespace,
                        ),
                        false,
                        0,
                    )
                }
                GeneratorState::Suspended { segment } => {
                    self.stack.extend(segment.stack);
                    self.exception_stack.extend(segment.exceptions);
                    let saved = segment.frame;
                    (
                        CallFrame {
                            code: &func.code,
                            bytecode: func.code.bytecode(),
                            ip: saved.ip,
                            stack_base: stack_index(stack_base),
                            locals_count: saved.locals_count,
                            exception_stack_base: stack_index(exception_base),
                            function_id: Some(function),
                            call_offset: Some(caller_offset),
                            should_return: false,
                            is_parked: false,
                            namespace: saved.namespace,
                            is_initializer: false,
                            import_from_module: saved.import_from_module,
                            generator_activation: None,
                        },
                        segment.delegating,
                        segment.fault_ip,
                    )
                }
                _ => unreachable!("validated before admission"),
            };
            frame.generator_activation = Some(GeneratorActivation {
                generator,
                native,
                default,
                delegated,
                delegating,
            });
            self.push_admitted_frame(frame);
            self.instruction_ip = fault_ip;
            if delegating {
                generator = self.peek().clone_with_heap(self.heap);
                default = None;
                delegated = true;
                continue;
            }
            if throwing {
                let error = self.make_exception(&input, false);
                return match self.handle_exception_with_value(error, Some(input)) {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }
            // New frames have no yield expression waiting for a send value.
            if self.current_frame.ip == 0 {
                input.drop_with(self);
            } else {
                self.push(input);
            }
            return Ok(());
        }
    }

    /// The delegate occupies one owning operand slot in the parent. Child
    /// activations carry only their pin and the persisted return-route flag.
    pub(super) fn delegate_generator(&mut self) -> RunResult<()> {
        let delegate = self.peek();
        if !matches!(delegate.read_heap(self), Some(HeapReadOutput::Generator(_))) {
            return Err(ExcType::not_implemented("yield from currently requires a generator delegate").into());
        }
        let generator = delegate.clone_with_heap(self.heap);
        self.current_frame
            .generator_activation
            .as_mut()
            .expect("generator parent")
            .delegating = true;
        let native = self
            .current_frame
            .generator_activation
            .as_ref()
            .expect("generator parent")
            .native;
        if let Err(error) = self.activate_generator_route(generator, Value::None, None, false, true, native) {
            self.current_frame
                .generator_activation
                .as_mut()
                .expect("generator parent")
                .delegating = false;
            return Err(error);
        }
        Ok(())
    }

    /// Remove the sole delegate operand and deliver one completed result owner.
    fn complete_delegation(&mut self, value: Value) {
        self.pop().drop_with(self);
        self.current_frame
            .generator_activation
            .as_mut()
            .expect("delegating parent")
            .delegating = false;
        self.push(value);
    }

    /// Move the current activation into its dormant heap payload, then deliver
    /// the yielded owner to the caller before releasing the activation pin.
    pub(super) fn suspend_generator(&mut self) -> RunResult<()> {
        loop {
            let Some(activation) = self.current_frame.generator_activation.take() else {
                return Err(RunError::internal("yield without generator activation"));
            };
            let yielded = self.pop();
            let stack_base = self.current_frame.stack_base();
            let exception_base = self.current_frame.exception_stack_base();
            let stack = self.stack.split_off(stack_base);
            let exceptions = self.exception_stack.split_off(exception_base);
            let mut frame = self.current_frame.serialize();
            frame.stack_base = 0;
            frame.exception_stack_base = 0;
            let segment = SavedGenerator {
                stack,
                frame,
                exceptions,
                fault_ip: self.instruction_ip,
                delegating: activation.delegating,
            };
            let caller_ip = self.current_frame.call_offset.expect("generator caller") as usize;
            self.pop_frame();
            self.instruction_ip = caller_ip;
            let Value::Ref(id) = &activation.generator else {
                unreachable!("activation pin")
            };
            {
                let HeapReadOutput::Generator(object) = self.heap.read(*id) else {
                    unreachable!("activation pin")
                };
                object.get_mut(self.heap).state = GeneratorState::Suspended { segment };
            }
            self.push(yielded);
            // Do not use activation.drop_with: it closes a *running* activation.
            activation.generator.drop_with(self);
            activation.default.drop_with(self);
            if !activation.delegated {
                return Ok(());
            }
        }
    }

    /// Close without retaining a result, transferring its sole owner directly
    /// to the delegating expression or the completion exception's args tuple.
    pub(super) fn finish_generator(&mut self, value: Value) -> RunResult<()> {
        let mut completion = DropGuard::new(value, self);
        let this = completion.ctx();
        let mut activation = this.current_frame.generator_activation.take().expect("generator frame");
        let default = activation.default.take();
        let delegated = activation.delegated;
        let native = activation.native;
        let caller_ip = this.current_frame.call_offset.expect("generator caller") as usize;
        activation.drop_with(this);
        this.pop_frame();
        this.instruction_ip = caller_ip;
        let (value, this) = completion.into_parts();
        if delegated {
            this.complete_delegation(value);
            default.drop_with(this);
            return Ok(());
        }
        if native {
            default.drop_with(this);
            this.push(value);
            return Ok(());
        }
        if let Some(default) = default {
            value.drop_with(this);
            this.push(default);
            Ok(())
        } else {
            Err(this.generator_completion_error(value))
        }
    }

    /// Build an owning completion without retaining any result on Closed.
    fn generator_completion_error(&mut self, value: Value) -> RunError {
        let values = if matches!(value, Value::None) {
            smallvec::smallvec![]
        } else {
            smallvec::smallvec![value]
        };
        let args = allocate_tuple(values, self.heap);
        let object = Value::Ref(self.heap.allocate(HeapData::Exception(Box::new(ExceptionObject {
            simple: SimpleException::new_none(ExcType::StopIteration),
            args: Some(args),
        }))));
        let error = self.make_exception(&object, false);
        object.drop_with(self);
        error
    }
}

#[cfg(all(test, feature = "ref-count-return"))]
#[path = "../../../tests/unit/generator_ownership.rs"]
mod tests;
