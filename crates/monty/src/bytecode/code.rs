//! Code object containing compiled bytecode and metadata.
//!
//! A `Code` object represents a compiled function or module. It contains the raw
//! bytecode instructions, a constant pool, source location information for tracebacks,
//! and an exception handler table.

use crate::{intern::StringId, parse::CodeRange, value::Value};

/// Compiled bytecode for a function or module.
///
/// This is the output of the bytecode compiler and the input to the VM.
/// Each function has its own Code object; module-level code also gets one.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Code {
    /// Variable-width instructions, addressed by body-relative offsets.
    #[serde(with = "serde_bytes")]
    #[serde(rename = "B")]
    bytecode: Vec<u8>,

    /// Immediate constants indexed by `LoadConst`; heap literals live in `Interns`.
    #[serde(rename = "C")]
    constants: Vec<Value>,

    /// Source location table for tracebacks.
    ///
    /// Maps bytecode offsets to source locations. Used to generate Python-style
    /// tracebacks with line numbers and caret markers when exceptions occur.
    #[serde(rename = "L")]
    location_table: Vec<LocationEntry>,

    /// Exception handler table.
    ///
    /// Maps protected bytecode ranges to their exception handlers. Consulted when
    /// an exception is raised to find the appropriate handler. Entries are ordered
    /// innermost-first for nested try blocks.
    #[serde(rename = "E")]
    exception_table: Vec<ExceptionEntry>,

    /// Local variable names for error messages.
    ///
    /// Maps slot indices to variable names. Used to generate proper NameError
    /// messages when accessing undefined local variables (e.g., "name 'x' is not defined").
    #[serde(rename = "N")]
    local_names: Vec<StringId>,

    /// Compiler-owned cleanup ranges. Only opt-in native cancellation consults these.
    #[serde(default, rename = "P")]
    pub(crate) native_cleanup: Option<Vec<(u32, u32)>>,

    /// Nop continuation boundaries after cleanup and non-local unwind.
    /// None identifies older code which cannot promise prompt native cancellation.
    #[serde(default, rename = "Q")]
    pub(crate) native_cancel_points: Option<Vec<u32>>,

    /// Distinguishes prospective except dispatch from required cleanup.
    /// Old code remains runnable but cannot admit the stronger cancellation policy.
    #[serde(default, rename = "R")]
    pub(crate) native_error_dispatch: bool,

    /// In-progress matching ranges and the frame-relative active exception index.
    /// None identifies code lacking cancellation arbitration across selector calls.
    #[serde(default, rename = "S")]
    pub(crate) native_matching: Option<Vec<(u32, u32, u16)>>,

    /// Semantic cancellation policy; absent in genuine legacy code.
    #[serde(default, rename = "T")]
    pub(crate) native_cancel_policy: u8,
}

impl Code {
    /// Creates an empty code object for tests that only need VM context.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
    }

    /// Creates a new Code object with all components.
    ///
    /// This is typically called by `CodeBuilder::build()` after compilation.
    #[must_use]
    pub fn new(
        bytecode: Vec<u8>,
        constants: Vec<Value>,
        location_table: Vec<LocationEntry>,
        exception_table: Vec<ExceptionEntry>,
        local_names: Vec<StringId>,
    ) -> Self {
        Self {
            bytecode,
            constants,
            location_table,
            exception_table,
            local_names,
            native_cleanup: Some(Vec::new()),
            native_cancel_points: Some(Vec::new()),
            native_error_dispatch: true,
            native_matching: Some(Vec::new()),
            native_cancel_policy: 7,
        }
    }

    /// Whether this code can participate in pending policy7 cancellation.
    pub(crate) fn supports_native_cancel(&self) -> bool {
        self.native_cancel_policy == 7
            && self.native_cancel_points.is_some()
            && self.native_error_dispatch
            && self.native_matching.is_some()
    }

    /// Whether a suspended instruction belongs to a lexical cleanup body.
    pub(crate) fn native_cleanup_at(&self, ip: usize) -> bool {
        self.native_cleanup
            .iter()
            .flatten()
            .any(|&(start, end)| (start as usize..end as usize).contains(&ip))
    }

    /// Resolve the existing exception owner; metadata never owns heap values.
    pub(crate) fn native_matching_at(&self, ip: usize) -> Option<usize> {
        self.native_matching.as_ref()?.iter().find_map(|&(start, end, depth)| {
            (start as usize..end as usize)
                .contains(&ip)
                .then_some(usize::from(depth))
        })
    }

    /// Returns the raw bytecode bytes.
    #[must_use]
    pub fn bytecode(&self) -> &[u8] {
        &self.bytecode
    }

    /// Returns the constant referenced by a `LoadConst` operand.
    /// Panics for an index not produced by this code's compiler.
    #[must_use]
    pub fn constant(&self, index: u16) -> &Value {
        &self.constants[usize::from(index)]
    }

    /// Returns the local variable name for a given slot index.
    ///
    /// Used to generate proper NameError messages when accessing undefined locals.
    #[must_use]
    pub fn local_name(&self, slot: u16) -> Option<StringId> {
        self.local_names.get(slot as usize).copied()
    }

    /// Finds the location entry for a given bytecode offset.
    ///
    /// Location entries are recorded at instruction boundaries. This method finds
    /// the most recent entry at or before the given offset.
    ///
    /// Returns `None` if the location table is empty, the offset is before
    /// the first recorded location, or the offset exceeds `u32::MAX` (an
    /// invariant violation; we degrade gracefully rather than panic since
    /// this is on the traceback hot path).
    #[must_use]
    pub fn location_for_offset(&self, offset: usize) -> Option<&LocationEntry> {
        let offset_u32 = u32::try_from(offset).ok()?;
        // Entries are sorted by bytecode offset: take the last at or before `offset`.
        let after = self
            .location_table
            .partition_point(|entry| entry.bytecode_offset <= offset_u32);
        after.checked_sub(1).map(|index| &self.location_table[index])
    }

    /// Finds an exception handler for the given bytecode offset.
    ///
    /// Searches the exception table for an entry whose protected range contains
    /// the given offset. Returns the first (innermost) matching handler, since
    /// entries are ordered innermost-first for nested try blocks.
    ///
    /// Pending native cancellation can skip prospective except dispatch outside
    /// cleanup, preserving the escaping error without executing matching code.
    /// Returns `None` if no eligible handler covers this offset.
    #[must_use]
    pub fn find_exception_handler(&self, offset: u32, skip_unprotected_except: bool) -> Option<&ExceptionEntry> {
        self.exception_table.iter().find(|entry| {
            entry.contains(offset)
                && !(skip_unprotected_except
                    && entry.kind == HandlerKind::Except
                    && !self.native_cleanup_at(entry.handler as usize))
        })
    }
}

impl Clone for Code {
    /// Constants are immediates, so copying compiled code needs no heap references.
    fn clone(&self) -> Self {
        Self {
            bytecode: self.bytecode.clone(),
            constants: self.constants.iter().map(Value::copy_immediate).collect(),
            location_table: self.location_table.clone(),
            exception_table: self.exception_table.clone(),
            local_names: self.local_names.clone(),
            native_cleanup: self.native_cleanup.clone(),
            native_cancel_points: self.native_cancel_points.clone(),
            native_error_dispatch: self.native_error_dispatch,
            native_matching: self.native_matching.clone(),
            native_cancel_policy: self.native_cancel_policy,
        }
    }
}

/// Source location for a bytecode instruction, used for tracebacks.
///
/// Python 3.11+ tracebacks show carets under the relevant expression:
/// ```text
///    File "test.py", line 2, in foo
///      return a + b + c
///             ~~^~~
/// ```
///
/// The `range` covers the full expression (`a + b`), while `focus` points
/// to the specific operator (`+`) that caused the error.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocationEntry {
    /// Bytecode offset this entry applies to.
    ///
    /// The entry applies from this offset until the next entry's offset
    /// (or end of bytecode).
    #[serde(rename = "B")]
    bytecode_offset: u32,

    /// Full source range of the expression (for the underline).
    #[serde(rename = "R")]
    range: CodeRange,

    /// Optional focus point within the range (for the ^ caret).
    ///
    /// If None, the entire range is underlined without a focus caret.
    /// This can be populated later for Python 3.11-style focused tracebacks.
    #[serde(rename = "F")]
    focus: Option<CodeRange>,
}

impl LocationEntry {
    /// Creates a new location entry.
    #[must_use]
    pub fn new(bytecode_offset: u32, range: CodeRange, focus: Option<CodeRange>) -> Self {
        Self {
            bytecode_offset,
            range,
            focus,
        }
    }

    /// Returns the full source range.
    #[must_use]
    pub fn range(&self) -> CodeRange {
        self.range
    }
}

/// Whether a handler reads the exception value from the operand stack.
/// Pushing it for a cleanup handler that only re-raises is wasted work on
/// every level an exception propagates through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HandlerKind {
    /// `with` cleanup (and legacy except dispatch), which reads the value.
    Consuming,
    /// Prospective except matching and body, which may execute arbitrary code.
    Except,
    /// Only re-raises: the VM skips the push, the compiler the matching `Pop`.
    Cleanup,
}

/// Entry in the exception table - maps a protected bytecode range to its handler.
///
/// Instead of maintaining a runtime stack of handlers (push/pop during execution),
/// we use a static table that's consulted when an exception is raised. This is
/// simpler and matches CPython 3.11+'s approach.
///
/// For nested try blocks, multiple entries may cover the same bytecode offset.
/// Entries are ordered innermost-first, so the VM uses the first matching entry.
///
/// # Example
///
/// For `try: x = bar(); y = baz() except ValueError as e: print(e)`:
/// ```text
/// 0:  LOAD_GLOBAL 'bar'
/// 4:  CALL_FUNCTION 0
/// 8:  STORE_LOCAL 'x'
/// ...
/// 24: JUMP 50              # skip handler if no exception
/// 30: <handler code>       # exception handler starts here
/// ```
/// Entry: `{ start: 0, end: 24, handler: 30, stack_depth: 0 }`
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ExceptionEntry {
    /// Start of protected bytecode range (inclusive).
    #[serde(rename = "S")]
    start: u32,

    /// End of protected bytecode range (exclusive).
    #[serde(rename = "E")]
    end: u32,

    /// Bytecode offset of the exception handler.
    #[serde(rename = "H")]
    handler: u32,

    /// Stack depth when entering the try block.
    ///
    /// Used to unwind the operand stack before jumping to handler.
    /// The VM pops values until the stack reaches this depth, then
    /// pushes the exception value.
    #[serde(rename = "D")]
    stack_depth: u16,

    /// This frame's `exception_stack` depth at region entry.
    /// Unwinding trims later entries so bare `raise` cannot revive exceptions
    /// from abandoned handlers.
    #[serde(rename = "C")]
    exception_stack_count: u16,

    /// Whether the handler wants the exception on the operand stack.
    kind: HandlerKind,
}

impl ExceptionEntry {
    /// Creates a new exception table entry.
    #[must_use]
    pub fn new(
        start: u32,
        end: u32,
        handler: u32,
        stack_depth: u16,
        exception_stack_count: u16,
        kind: HandlerKind,
    ) -> Self {
        Self {
            start,
            end,
            handler,
            stack_depth,
            exception_stack_count,
            kind,
        }
    }

    /// Whether the VM must push the exception value before entering the handler.
    #[must_use]
    pub fn pushes_exception(&self) -> bool {
        matches!(self.kind, HandlerKind::Consuming | HandlerKind::Except)
    }

    /// Returns the handler bytecode offset.
    #[must_use]
    pub fn handler(&self) -> u32 {
        self.handler
    }

    /// A prospective user handler, as opposed to required unwind bookkeeping.
    pub(crate) fn is_except(&self) -> bool {
        self.kind == HandlerKind::Except
    }

    /// Returns the stack depth to unwind to.
    #[must_use]
    pub fn stack_depth(&self) -> u16 {
        self.stack_depth
    }

    /// Returns this frame's exception-stack depth at region entry.
    #[must_use]
    pub fn exception_stack_count(&self) -> u16 {
        self.exception_stack_count
    }

    /// Returns true if the given bytecode offset is within this entry's protected range.
    #[must_use]
    pub fn contains(&self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }
}
