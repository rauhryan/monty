//! Host-only, one-shot instrumentation for the repair10 acceptance tests.
//!
//! Absent unless `cancellation-test-hooks` is explicitly enabled. No guest builtin,
//! snapshot field, protocol request, or production configuration selects a fault.
//! State is thread-local and is deliberately not serialized: restored tests must
//! re-arm it from their trusted host. Instrumented workers are not distributable.
use std::cell::RefCell;

use monty_types::MontyException;

use crate::{bytecode::VM, exception_private::RunError};

#[derive(Default)]
struct State {
    mode: String,
    export_ready: bool,
    stats: ExportStats,
}

/// Live exporter observations; counts are taken before REPL/worker teardown.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExportStats {
    pub visits: usize,
    pub peak_pins: usize,
    pub retired_pins: usize,
    pub dropped: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Arm one fault on this host thread. Empty disables instrumentation.
pub fn configure(mode: &str) {
    assert!(matches!(
        mode,
        "" | "depth"
            | "terminal"
            | "delivery-internal"
            | "delivery-uncatchable"
            | "projection-internal"
            | "projection-uncatchable"
    ));
    STATE.with_borrow_mut(|s| {
        *s = State {
            mode: mode.into(),
            ..State::default()
        }
    });
}

/// Read observations without resetting them.
#[must_use]
pub fn export_stats() -> ExportStats {
    STATE.with_borrow(|s| s.stats)
}

fn error_at(site: &str) -> Option<RunError> {
    STATE.with_borrow_mut(|s| {
        let kind = s.mode.strip_prefix(site)?.strip_prefix('-')?;
        let error = match kind {
            "internal" => RunError::internal("REPAIR10_INTERNAL"),
            "uncatchable" => RunError::uncatchable(MontyException::runtime_error("REPAIR10_UNCATCHABLE")),
            _ => return None,
        };
        eprintln!("repair10-fault: {}", s.mode);
        s.mode.clear();
        Some(error)
    })
}

pub(crate) fn delivery_error() -> Option<RunError> {
    error_at("delivery")
}
pub(crate) fn projection_error() -> Option<RunError> {
    error_at("projection")
}

pub(crate) fn prepare_export(vm: &mut VM<'_>) {
    STATE.with_borrow_mut(|s| {
        if s.mode == "depth" {
            // Exercise the real recursion_guard / DepthLimit branch, not a fake
            // export result. Tightening happens only at cancelled return export.
            vm.heap.tracker.lower_recursion_limit(16).expect("test ceiling");
        }
        s.export_ready = matches!(s.mode.as_str(), "depth" | "terminal");
    });
}

pub(crate) fn take_export() -> bool {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.export_ready))
}
pub(crate) fn visit(pins: usize) {
    STATE.with_borrow_mut(|s| {
        s.stats.visits += 1;
        s.stats.peak_pins = s.stats.peak_pins.max(pins);
    });
}
pub(crate) fn terminal_due(pins: usize) -> bool {
    STATE.with_borrow(|s| s.mode == "terminal" && pins >= 1)
}
pub(crate) fn retired(pins: usize) {
    STATE.with_borrow_mut(|s| {
        s.stats.retired_pins = pins;
        s.stats.dropped = true;
        eprintln!("repair10-export: {:?}", s.stats);
        s.mode.clear();
    });
}
