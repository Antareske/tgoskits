//! Network queue event adapters.
//!
//! The queue runtime owns the network facts and reports them through a narrow
//! observation port; this module owns the `net:*` tracepoint name, the record
//! layout and the published gate that lets the runtime skip reports while no
//! consumer is attached.

use ax_net::QueuePollReport;

/// Converts a runtime value to its recorded width.
///
/// The runtime bounds these values far below `u32::MAX` (device counts, CPU
/// ids, the round budget and its work units), so a saturated field means the
/// runtime contract changed rather than that a meaningful value was truncated.
fn field(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

ax_tracepoint::define_event_trace!(
    queue_poll_round,
    TP_kops(crate::tracepoint::KernelTraceAux),
    TP_system(net),
    TP_PROTO(
        discovery_order: u32,
        group_id: u32,
        owner_cpu: u32,
        budget: u32,
        work_units: u32,
        outcome: u32,
    ),
    TP_STRUCT__entry {
        discovery_order: u32,
        group_id: u32,
        owner_cpu: u32,
        budget: u32,
        work_units: u32,
        outcome: u32,
    },
    TP_fast_assign {
        discovery_order: discovery_order,
        group_id: group_id,
        owner_cpu: owner_cpu,
        budget: budget,
        work_units: work_units,
        outcome: outcome,
    },
    TP_ident(__entry),
    TP_printk({
        alloc::format!(
            "discovery_order={} group_id={} owner_cpu={} budget={} work_units={} outcome={}",
            __entry.discovery_order,
            __entry.group_id,
            __entry.owner_cpu,
            __entry.budget,
            __entry.work_units,
            __entry.outcome,
        )
    })
);

/// Reports one completed queue executor poll round.
///
/// The runtime calls this from the queue executor thread.  The values are the
/// port's domain facts; the generated event function re-checks the gate and
/// only then builds the record.
fn on_queue_poll(report: QueuePollReport) {
    trace_queue_poll_round(
        field(report.identity.discovery_order),
        u32::from(report.identity.group_id.get()),
        field(report.identity.owner_cpu),
        field(report.budget),
        field(report.work_units),
        report.outcome as u32,
    );
}

/// Installs the observation port and publishes the initial gate.
///
/// The network runtime is already running here, so rounds that completed
/// before this call are not reported.  The gate starts from the tracepoint's
/// own state, which is "no callbacks" until a consumer attaches.
pub(super) fn install() {
    ax_net::install_queue_poll_observer(on_queue_poll);
    super::gate::register(&__queue_poll_round, ax_net::publish_queue_poll_gate);
}
