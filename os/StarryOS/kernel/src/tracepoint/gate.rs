//! Runtime gate sinks for events whose facts are produced outside this module.
//!
//! An event's enable state lives in its callback set, which the tracepoint
//! registry owns.  A layer that produces an event on its own hot path cannot
//! read that state, so it keeps a published copy and skips the record while no
//! consumer is attached.  The module that defines such an event registers the
//! publishing function of that layer here; the registry mirrors every
//! callback-set change to it, so adding an event stays local to the module
//! that defines it.

use alloc::vec::Vec;

use ax_tracepoint::TracePoint;

use super::KernelTraceAux;
use crate::sync::Mutex;

/// Publishes one event's enable state to the layer that produces it.
type GateSink = fn(bool);

static GATE_SINKS: Mutex<Vec<(&'static TracePoint<KernelTraceAux>, GateSink)>> =
    Mutex::new(Vec::new());

/// Registers the gate sink of `tracepoint` and publishes its current state.
///
/// Registration happens during tracepoint initialization, before any consumer
/// can change a callback set.
pub(super) fn register(tracepoint: &'static TracePoint<KernelTraceAux>, sink: GateSink) {
    GATE_SINKS.lock().push((tracepoint, sink));
    sink(tracepoint.key_is_enabled());
}

/// Mirrors a callback-set change to the sink registered for `tracepoint`.
///
/// The registry calls this from the same update that changes the callback set,
/// so a tracefs `enable` write and a perf/BPF attach both reach the producing
/// layer.
pub(super) fn publish(tracepoint: &'static TracePoint<KernelTraceAux>, enabled: bool) {
    for (registered, sink) in GATE_SINKS.lock().iter() {
        if core::ptr::eq(*registered, tracepoint) {
            sink(enabled);
        }
    }
}
