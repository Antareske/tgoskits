//! Narrow observation port for queue runtime events.
//!
//! The port is one typed function-pointer slot installed by the OS adapter and
//! never replaced or removed.  A report costs one published-flag load when no
//! consumer is active, and the slot load plus the call when one is; it never
//! allocates, reads a clock or takes a network lock, so it is safe on the
//! queue executor path.

use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use super::NetQueueIdentity;

/// Result of one queue executor poll round.
///
/// The discriminants are the reported codes and are part of the event
/// contract: they must not be reordered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum QueuePollOutcome {
    /// The round found no further work.
    Idle    = 0,
    /// Work remains: a budget was exhausted or retryable RX work is pending.
    More    = 1,
    /// The round stopped because a ring or replacement buffer was not ready.
    Blocked = 2,
    /// The round failed; the group is disabled.
    Failed  = 3,
}

/// One completed queue executor poll round.
///
/// The identity is the one fixed when the group was built, so `owner_cpu` is
/// the group's owner rather than a CPU sampled at report time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueuePollReport {
    pub identity: NetQueueIdentity,
    /// CPU work budget handed to this poll call.  It is the round's remaining
    /// budget, so it shrinks as the round serves earlier groups.
    pub budget: usize,
    /// Executor work units completed by this call.  A unit is one completed
    /// TX reclaim, TX submit, RX recycle, RX refill or RX reclaim; it is not a
    /// frame count.
    pub work_units: usize,
    pub outcome: QueuePollOutcome,
}

/// Consumer of completed queue poll rounds.
pub type QueuePollObserver = fn(QueuePollReport);

static QUEUE_POLL_OBSERVER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static QUEUE_POLL_ENABLED: AtomicBool = AtomicBool::new(false);

/// Installs the process-wide queue poll consumer.
///
/// Reinstalling the same function is harmless; replacing a live consumer is an
/// invariant violation because poll rounds may concurrently execute it.  The
/// port is installed after the runtime is running, so rounds that complete
/// before installation are not reported.
pub fn install_queue_poll_observer(observer: QueuePollObserver) {
    let observer = observer as *mut ();
    match QUEUE_POLL_OBSERVER.compare_exchange(
        core::ptr::null_mut(),
        observer,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(installed) => assert_eq!(installed, observer, "queue poll observer already installed"),
    }
}

/// Publishes whether the queue poll event has active consumers.
///
/// The tracepoint adapter owns the authoritative gate and mirrors it here, so
/// the runtime can skip a report without querying the tracepoint registry.
/// The flag and the slot are read separately, so a report that races with a
/// gate change may still reach the consumer; the generated event function
/// repeats the gate check as the final authority.
pub fn publish_queue_poll_gate(enabled: bool) {
    QUEUE_POLL_ENABLED.store(enabled, Ordering::Release);
}

pub(super) fn report_queue_poll(report: QueuePollReport) {
    if !QUEUE_POLL_ENABLED.load(Ordering::Acquire) {
        return;
    }
    let observer = QUEUE_POLL_OBSERVER.load(Ordering::Acquire);
    if observer.is_null() {
        return;
    }
    // SAFETY: installation accepts exactly this function-pointer type, and the
    // slot is never replaced or removed.
    let observer = unsafe { core::mem::transmute::<*mut (), QueuePollObserver>(observer) };
    observer(report);
}
