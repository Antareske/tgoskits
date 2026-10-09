#![no_std]

//! The counter table both halves of the tool share.
//!
//! The eBPF programs increment one slot per observed fact; the loader sums each
//! slot across CPUs and prints it.  Slots are grouped in fixed `SLOT_STRIDE`
//! blocks, one block per `net:*` event, so an event can grow without moving its
//! neighbours' offsets.  The layout is shared ABI: changing a slot's meaning
//! means changing both halves in the same edit.

/// Slots reserved per event block.
pub const SLOT_STRIDE: u32 = 16;

/// Slots per CPU region: one block per attached event.
pub const COUNTER_SLOTS: u32 = 6 * SLOT_STRIDE;

/// Owner CPUs the table keeps separate regions for.
///
/// The counter map is a plain array rather than a `BPF_MAP_TYPE_PERCPU_ARRAY`
/// on purpose.  This kernel's per-CPU map reads hand back the *reading* CPU's
/// slice — `PerCpuArrayMap::lookup_elem` goes through `PerCpuVariants::get_mut`,
/// which indexes by `this_cpu_id()` — so a userspace reader that sums what it
/// read would be summing whichever CPU happened to service the syscall.  A
/// plain array reads the same from every CPU, and keeping one region per owner
/// CPU keeps each slot written by a single CPU, which is what makes the
/// read-modify-write in the eBPF program safe without atomics (this kernel has
/// no atomic map instructions either).
///
/// An event's region comes from the `owner_cpu` field it carries, not from the
/// CPU running the eBPF program: the queue executor reports on its group's
/// owner CPU by construction.  Folding with `%` means a CPU beyond this count
/// shares a region instead of losing its events.
pub const PARTITIONS: u32 = 8;

/// Entries in the counter map: one region per owner CPU.
pub const TABLE_SLOTS: u32 = PARTITIONS * COUNTER_SLOTS;

/// First index of the region belonging to `cpu`.
pub const fn region(cpu: u32) -> u32 {
    (cpu % PARTITIONS) * COUNTER_SLOTS
}

/// First slot of each event's block.
pub const POLL_BASE: u32 = 0;
pub const REARM_BASE: u32 = SLOT_STRIDE;
pub const BACKPRESSURE_BASE: u32 = 2 * SLOT_STRIDE;
pub const TX_SUBMIT_BASE: u32 = 3 * SLOT_STRIDE;
pub const RX_PUBLISH_BASE: u32 = 4 * SLOT_STRIDE;
pub const PROTO_YIELD_BASE: u32 = 5 * SLOT_STRIDE;

/// Every block's first slot: how many records of that event were seen.
pub const TOTAL: u32 = 0;

// `net:queue_poll_round`
pub const POLL_IDLE: u32 = 1;
pub const POLL_MORE: u32 = 2;
pub const POLL_BLOCKED: u32 = 3;
pub const POLL_FAILED: u32 = 4;
/// Sum of `work_units` over the rounds; the loader divides by `TOTAL`.
pub const POLL_WORK_UNITS: u32 = 5;

// `net:queue_rearm`
pub const REARM_RACE: u32 = 1;
pub const REARM_WORK_PENDING: u32 = 2;
pub const REARM_RETRY_AT: u32 = 3;
pub const REARM_FAILED: u32 = 4;

// `net:queue_backpressure`
pub const BACKPRESSURE_TX_RETRY: u32 = 1;
pub const BACKPRESSURE_TX_LINK_DOWN: u32 = 2;
pub const BACKPRESSURE_RX_RETRY: u32 = 3;
/// Unreachable by construction (RX refill reports a link-down refusal as a
/// failed round), kept so the slot numbering matches the (stage, reason)
/// matrix the event documents.
pub const BACKPRESSURE_RX_LINK_DOWN: u32 = 4;

// `net:tx_submit` and `net:rx_publish`
/// Sum of the frame lengths.  The loader divides the window's growth by the
/// window's frame count: a per-run minimum or maximum would describe the whole
/// run rather than the window the reading is about.
pub const FRAME_BYTES: u32 = 1;

// `net:proto_yield`
pub const YIELD_POLL_COUNT: u32 = 1;
pub const YIELD_DEADLINE: u32 = 2;
pub const YIELD_BOTH: u32 = 3;
/// Yields that left work pending: the saturation signal.
pub const YIELD_WORK_PENDING: u32 = 4;

/// What an event's block means, so the loader renders each one in its own
/// terms rather than printing a generic column of numbers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Poll,
    Rearm,
    Backpressure,
    TxSubmit,
    RxPublish,
    ProtoYield,
}

/// One `net:*` event the tool attaches to.
pub struct EventSpec {
    /// Name under `events/net/`, and the name the program is attached by.
    pub name: &'static str,
    /// The record's fields in the order the kernel writes them, used to check
    /// the tracepoint's own `format` before any offset is trusted.
    pub fields: &'static [&'static str],
    /// First slot of this event's counter block.
    pub base: u32,
    /// How the block's slots are read.
    pub kind: EventKind,
}

/// The six events this tool exists for, in slot order.
pub const EVENTS: [EventSpec; 6] = [
    EventSpec {
        name: "queue_poll_round",
        fields: &[
            "discovery_order",
            "group_id",
            "owner_cpu",
            "budget",
            "work_units",
            "outcome",
        ],
        base: POLL_BASE,
        kind: EventKind::Poll,
    },
    EventSpec {
        name: "queue_rearm",
        fields: &["discovery_order", "group_id", "owner_cpu", "outcome"],
        base: REARM_BASE,
        kind: EventKind::Rearm,
    },
    EventSpec {
        name: "queue_backpressure",
        fields: &[
            "discovery_order",
            "group_id",
            "owner_cpu",
            "stage",
            "reason",
        ],
        base: BACKPRESSURE_BASE,
        kind: EventKind::Backpressure,
    },
    EventSpec {
        name: "tx_submit",
        fields: &["discovery_order", "group_id", "owner_cpu", "frame_len"],
        base: TX_SUBMIT_BASE,
        kind: EventKind::TxSubmit,
    },
    EventSpec {
        name: "rx_publish",
        fields: &["discovery_order", "group_id", "owner_cpu", "frame_len"],
        base: RX_PUBLISH_BASE,
        kind: EventKind::RxPublish,
    },
    EventSpec {
        name: "proto_yield",
        fields: &["owner_cpu", "reason", "work_pending"],
        base: PROTO_YIELD_BASE,
        kind: EventKind::ProtoYield,
    },
];
