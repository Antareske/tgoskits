//! Counts the six `net:*` tracepoints into one counter table.
//!
//! Every program here is straight-line: the kernel attaches only programs whose
//! control flow advances through the instruction stream at most once, so a
//! compiled loop would be rejected at load time.  Each program reads the fields
//! it needs from the tracepoint context by offset and folds them into the
//! region of the table that belongs to the CPU the event names.

#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{map, tracepoint},
    maps::Array,
    programs::TracePointContext,
};
use net_events_common::*;

/// The counter table: one region per owner CPU, one block per event in each
/// region.
///
/// A plain array, not a per-CPU map: the read path of a per-CPU map on this
/// kernel returns the reading CPU's slice (see `net_events-common`), and this
/// kernel has no atomic map instructions to make a shared counter safe.  A
/// region per owner CPU gives the same safety — every slot has one writer —
/// while staying readable from any CPU.
#[map]
static COUNTERS: Array<u64> = Array::<u64>::with_max_entries(TABLE_SLOTS, 0);

/// Bytes of the generic trace entry the kernel prepends to every record:
/// `u16 common_type`, `u8 common_flags`, `u8 common_preempt_count`,
/// `i32 common_pid`.  The event's own fields follow it in `format` order.
const COMMON_LEN: usize = 8;

/// Add `delta` to one entry of the table.
#[inline(always)]
fn add(index: u32, delta: u64) {
    if let Some(cell) = COUNTERS.get_ptr_mut(index) {
        // SAFETY: `get_ptr_mut` returns a valid, properly aligned pointer into
        // the map value, which the verifier checks before the program loads.
        // Each entry is written by the single CPU that owns its region, so the
        // read-modify-write races with nothing.
        unsafe { *cell += delta };
    }
}

/// Read a `u32` field of the record.
#[inline(always)]
fn field(ctx: &TracePointContext, offset: usize) -> u32 {
    // SAFETY: the offsets follow each event's declared field order, which the
    // loader checks against the tracepoint's own `format` before attaching.
    // `unwrap_or(0)` covers a record truncated by the verifier's bounds check.
    unsafe { ctx.read_at::<u32>(offset).unwrap_or(0) }
}

/// `net:queue_poll_round` — one completed queue executor poll round.
#[tracepoint]
pub fn queue_poll_round(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN + 8));
    let work_units = field(&ctx, COMMON_LEN + 16) as u64;
    let outcome = field(&ctx, COMMON_LEN + 20);

    add(at + POLL_BASE + TOTAL, 1);
    add(at + POLL_BASE + POLL_WORK_UNITS, work_units);
    if outcome == 0 {
        add(at + POLL_BASE + POLL_IDLE, 1);
    } else if outcome == 1 {
        add(at + POLL_BASE + POLL_MORE, 1);
    } else if outcome == 2 {
        add(at + POLL_BASE + POLL_BLOCKED, 1);
    } else if outcome == 3 {
        add(at + POLL_BASE + POLL_FAILED, 1);
    }
    0
}

/// `net:queue_rearm` — a rearm that did not end idle.  Idle rearms are not
/// reported at all, so this counts the exceptions only.
#[tracepoint]
pub fn queue_rearm(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN + 8));
    let outcome = field(&ctx, COMMON_LEN + 12);

    add(at + REARM_BASE + TOTAL, 1);
    if outcome == 0 {
        add(at + REARM_BASE + REARM_RACE, 1);
    } else if outcome == 1 {
        add(at + REARM_BASE + REARM_WORK_PENDING, 1);
    } else if outcome == 2 {
        add(at + REARM_BASE + REARM_RETRY_AT, 1);
    } else if outcome == 3 {
        add(at + REARM_BASE + REARM_FAILED, 1);
    }
    0
}

/// `net:queue_backpressure` — the device asked to be waited for.
#[tracepoint]
pub fn queue_backpressure(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN + 8));
    let stage = field(&ctx, COMMON_LEN + 12);
    let reason = field(&ctx, COMMON_LEN + 16);

    add(at + BACKPRESSURE_BASE + TOTAL, 1);
    if stage == 0 && reason == 0 {
        add(at + BACKPRESSURE_BASE + BACKPRESSURE_TX_RETRY, 1);
    } else if stage == 0 && reason == 1 {
        add(at + BACKPRESSURE_BASE + BACKPRESSURE_TX_LINK_DOWN, 1);
    } else if stage == 1 && reason == 0 {
        add(at + BACKPRESSURE_BASE + BACKPRESSURE_RX_RETRY, 1);
    } else if stage == 1 && reason == 1 {
        add(at + BACKPRESSURE_BASE + BACKPRESSURE_RX_LINK_DOWN, 1);
    }
    0
}

/// `net:tx_submit` — the driver accepted one frame.
#[tracepoint]
pub fn tx_submit(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN + 8));
    let frame_len = field(&ctx, COMMON_LEN + 12) as u64;

    add(at + TX_SUBMIT_BASE + TOTAL, 1);
    add(at + TX_SUBMIT_BASE + FRAME_BYTES, frame_len);
    0
}

/// `net:rx_publish` — one frame was published to the protocol side.
#[tracepoint]
pub fn rx_publish(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN + 8));
    let frame_len = field(&ctx, COMMON_LEN + 12) as u64;

    add(at + RX_PUBLISH_BASE + TOTAL, 1);
    add(at + RX_PUBLISH_BASE + FRAME_BYTES, frame_len);
    0
}

/// `net:proto_yield` — the protocol executor handed the CPU over because a
/// budget ran out.
#[tracepoint]
pub fn proto_yield(ctx: TracePointContext) -> u32 {
    let at = region(field(&ctx, COMMON_LEN));
    let reason = field(&ctx, COMMON_LEN + 4);
    let work_pending = field(&ctx, COMMON_LEN + 8);

    add(at + PROTO_YIELD_BASE + TOTAL, 1);
    if reason == 0 {
        add(at + PROTO_YIELD_BASE + YIELD_POLL_COUNT, 1);
    } else if reason == 1 {
        add(at + PROTO_YIELD_BASE + YIELD_DEADLINE, 1);
    } else if reason == 2 {
        add(at + PROTO_YIELD_BASE + YIELD_BOTH, 1);
    }
    if work_pending != 0 {
        add(at + PROTO_YIELD_BASE + YIELD_WORK_PENDING, 1);
    }
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // The verifier rejects loops, so a spinning handler would be rejected at
    // load time; mark it unreachable as the other in-tree programs do.
    unsafe { core::hint::unreachable_unchecked() }
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
