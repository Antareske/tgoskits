//! Temporary board-measurement probe for the TX/RX data plane.
//!
//! Diagnostic only; nothing here changes device behaviour.  Counters accumulate
//! inside a window and are reported on the serial console when the window
//! closes, so one report describes one window instead of a running total.
//!
//! This round prices the work that shares the owner and the SDIO bus with a
//! transmit packet, because shortening the interval between two writes is only
//! worth doing where that interval is software round trip and not traffic that
//! has to happen anyway:
//!
//! - every transaction is booked by class, so the receive drain is visible next
//!   to the transmit write;
//! - `gap` splits the interval between two write completions by how many
//!   receive transactions were consumed inside it, and by where the next frame
//!   was when the previous write completed;
//! - `supply` and `rdif_at_write` record whether a frame was already waiting in
//!   the core queue or in the RDIF ring, which separates "the stack produced
//!   nothing" from "a frame was waiting for a pull".
//!
//! Timestamps come from the caller's monotonic clock (`AicInput::now`), so one
//! owner call carries a single timestamp and every interval is measured between
//! call boundaries.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::{IoPurpose, MonotonicTime, SdioRequestKind};

/// Window flush period, matching the queue-executor probe so both logs can be
/// read side by side.
const WINDOW_NANOS: u64 = 2_000_000_000;

/// Transaction classes booked per window.
const TX_WRITE: usize = 0;
const TX_CREDIT: usize = 1;
const RX_DATA: usize = 2;
const RX_CONTROL: usize = 3;
const OTHER: usize = 4;
const CLASSES: usize = 5;

/// Receive transactions consumed inside one write-to-write gap: none, one, or
/// more.
const GAP_BUCKETS: usize = 3;
/// Receive read length: up to one block, up to 2 KiB, up to 8 KiB, more.
const RX_SIZE_BUCKETS: usize = 4;
/// Packet period: below the 0.25 ms target, then 0.6 / 1.0 / 1.5 ms, then more.
const PERIOD_BUCKETS: usize = 5;
/// Queue occupancy: empty, one, two to three, four or more.
const DEPTH_BUCKETS: usize = 4;
/// Where the next frame was when a write completed: core queue, RDIF ring, or
/// neither.
const SUPPLY_BUCKETS: usize = 3;
const SUPPLY_CORE: usize = 0;
const SUPPLY_RING: usize = 1;
const SUPPLY_NONE: usize = 2;

/// Depth was not sampled for this completion.
const NO_DEPTH: usize = usize::MAX;

/// Owner loop iterations, to price the per-packet scheduling cost.
pub(crate) static OWNER_STEPS: AtomicU64 = AtomicU64::new(0);
/// Owner entry points, to count how often the core is advanced at all.
pub(crate) static OWNER_CALLS: AtomicU64 = AtomicU64::new(0);
/// RDIF transmit ring occupancy sampled when an SDIO operation completed.
static RDIF_TX_DEPTH: AtomicUsize = AtomicUsize::new(NO_DEPTH);

#[cfg(feature = "rdif")]
pub(crate) fn owner_step() {
    OWNER_STEPS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "rdif")]
pub(crate) fn owner_call() {
    OWNER_CALLS.fetch_add(1, Ordering::Relaxed);
}

/// Records how many frames the RDIF ring held when the SDIO operation that just
/// completed was issued to the core.
#[cfg(feature = "rdif")]
pub(crate) fn note_rdif_tx_depth(depth: usize) {
    RDIF_TX_DEPTH.store(depth, Ordering::Relaxed);
}

/// Takes the depth noted for the completion being consumed.
pub(crate) fn take_depth() -> Option<usize> {
    match RDIF_TX_DEPTH.swap(NO_DEPTH, Ordering::Relaxed) {
        NO_DEPTH => None,
        depth => Some(depth),
    }
}

/// One SDIO transaction between being handed to the owner and completing.
struct InFlight {
    class: usize,
    start: MonotonicTime,
    bytes: u64,
}

#[derive(Default)]
pub(super) struct TxProbe {
    /// Timestamp of the advance call currently being processed.
    pub(super) now: MonotonicTime,
    window_open: Option<MonotonicTime>,
    owner_steps_at_open: u64,
    owner_calls_at_open: u64,
    in_flight: Option<InFlight>,
    /// Per-class round trips.
    class_count: [u64; CLASSES],
    class_bytes: [u64; CLASSES],
    class_nanos: [u64; CLASSES],
    class_max: [u64; CLASSES],
    rx_size_count: [u64; RX_SIZE_BUCKETS],
    rx_size_nanos: [u64; RX_SIZE_BUCKETS],
    /// Firmware flow-control readings.
    credit_samples: u64,
    credit_sum: u64,
    credit_min: u8,
    credit_max: u8,
    /// Backoffs taken at the command reserve, and how long they waited.
    backoffs: u64,
    backoff_since: Option<MonotonicTime>,
    backoff_wait_nanos: u64,
    /// Packet periods.
    packets: u64,
    last_write_done: Option<MonotonicTime>,
    period_nanos: u64,
    period_max: u64,
    period_count: [u64; PERIOD_BUCKETS],
    /// Frames parsed out of receive reads.
    rx_frames: u64,
    /// Receive transactions consumed since the previous write completed.
    rx_since_write: u64,
    gap_count: [u64; GAP_BUCKETS],
    gap_nanos: [u64; GAP_BUCKETS],
    /// Where the next frame was when the previous write completed, and the gap
    /// that followed.
    supply_core: bool,
    supply_ring: bool,
    supply_count: [u64; SUPPLY_BUCKETS],
    supply_nanos: [u64; SUPPLY_BUCKETS],
    /// Queue depths sampled at a write completion and at a frame arrival.
    rdif_at_write: [u64; DEPTH_BUCKETS],
    queue_at_arrival: [u64; DEPTH_BUCKETS],
}

impl TxProbe {
    pub(super) fn set_now(&mut self, now: MonotonicTime) {
        self.now = now;
        if self.window_open.is_none() {
            self.open_window(now);
        }
    }

    /// Closes a window that has run its period, so slow windows are still
    /// reported.
    pub(super) fn flush_window(&mut self) {
        let Some(window) = self.window_open else {
            return;
        };
        if self.now.as_nanos().saturating_sub(window.as_nanos()) < WINDOW_NANOS {
            return;
        }
        // An idle window would only print zeros, so it is closed silently.
        if self.packets == 0 && self.class_count.iter().all(|count| *count == 0) {
            self.open_window(self.now);
            return;
        }
        self.report();
    }

    /// Books a transaction as it is handed to the SDIO owner.
    pub(super) fn emitted(&mut self, purpose: &IoPurpose, kind: &SdioRequestKind) {
        let class = classify(purpose);
        if class == TX_WRITE {
            self.close_gap();
        }
        self.in_flight = Some(InFlight {
            class,
            start: self.now,
            bytes: request_bytes(kind),
        });
    }

    /// Books the completion of the transaction that was in flight.
    pub(super) fn completed(&mut self, purpose: &IoPurpose) {
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };
        let rtt = self
            .now
            .as_nanos()
            .saturating_sub(in_flight.start.as_nanos());
        self.class_count[in_flight.class] += 1;
        self.class_bytes[in_flight.class] += in_flight.bytes;
        self.class_nanos[in_flight.class] += rtt;
        self.class_max[in_flight.class] = self.class_max[in_flight.class].max(rtt);
        match classify(purpose) {
            RX_DATA => {
                let bucket = rx_size_bucket(in_flight.bytes);
                self.rx_size_count[bucket] += 1;
                self.rx_size_nanos[bucket] += rtt;
                self.rx_since_write += 1;
            }
            RX_CONTROL => self.rx_since_write += 1,
            _ => {}
        }
    }

    /// Drops the in-flight record when an operation is abandoned instead of
    /// completing, so a cancelled transaction never books a round trip.
    pub(super) fn abandon(&mut self) {
        self.in_flight = None;
    }

    /// Books a completed transmit write: the packet period it closes and where
    /// the next frame already was.
    pub(super) fn write_done(&mut self, core_ready: bool, rdif_depth: Option<usize>) {
        if let Some(depth) = rdif_depth {
            self.rdif_at_write[depth_bucket(depth)] += 1;
        }
        self.supply_core = core_ready;
        self.supply_ring = rdif_depth.is_some_and(|depth| depth > 0);
        if let Some(previous) = self.last_write_done {
            let period = self.now.as_nanos().saturating_sub(previous.as_nanos());
            self.period_nanos += period;
            self.period_max = self.period_max.max(period);
            self.period_count[period_bucket(period)] += 1;
        }
        self.last_write_done = Some(self.now);
        self.packets += 1;
    }

    pub(super) fn credit(&mut self, credits: u8) {
        if self.credit_samples == 0 {
            self.credit_min = credits;
            self.credit_max = credits;
        }
        self.credit_samples += 1;
        self.credit_sum += u64::from(credits);
        self.credit_min = self.credit_min.min(credits);
        self.credit_max = self.credit_max.max(credits);
    }

    pub(super) fn credit_backoff(&mut self, now: MonotonicTime) {
        self.backoffs += 1;
        self.backoff_since = Some(now);
    }

    /// Ends a credit wait window at the first successful read.
    pub(super) fn credit_read(&mut self) {
        if let Some(start) = self.backoff_since.take() {
            self.backoff_wait_nanos += self.now.as_nanos().saturating_sub(start.as_nanos());
        }
    }

    /// Records how many frames a receive read parsed.
    pub(super) fn rx_parsed(&mut self, frames: usize) {
        self.rx_frames += frames as u64;
    }

    /// Records the core queue depth a frame arrived at.
    pub(super) fn frame_arrived(&mut self, depth: usize) {
        self.queue_at_arrival[depth_bucket(depth)] += 1;
    }

    /// Closes the gap that the previous write completion opened.
    fn close_gap(&mut self) {
        let rx = self.rx_since_write.min((GAP_BUCKETS - 1) as u64) as usize;
        self.rx_since_write = 0;
        let Some(previous) = self.last_write_done else {
            return;
        };
        let gap = self.now.as_nanos().saturating_sub(previous.as_nanos());
        self.gap_count[rx] += 1;
        self.gap_nanos[rx] += gap;
        let supply = if self.supply_core {
            SUPPLY_CORE
        } else if self.supply_ring {
            SUPPLY_RING
        } else {
            SUPPLY_NONE
        };
        self.supply_count[supply] += 1;
        self.supply_nanos[supply] += gap;
    }

    fn elapsed(&self, start: MonotonicTime) -> u64 {
        self.now.as_nanos().saturating_sub(start.as_nanos())
    }

    fn open_window(&mut self, now: MonotonicTime) {
        self.window_open = Some(now);
        self.owner_steps_at_open = OWNER_STEPS.load(Ordering::Relaxed);
        self.owner_calls_at_open = OWNER_CALLS.load(Ordering::Relaxed);
        self.class_count = [0; CLASSES];
        self.class_bytes = [0; CLASSES];
        self.class_nanos = [0; CLASSES];
        self.class_max = [0; CLASSES];
        self.rx_size_count = [0; RX_SIZE_BUCKETS];
        self.rx_size_nanos = [0; RX_SIZE_BUCKETS];
        self.credit_samples = 0;
        self.credit_sum = 0;
        self.credit_min = 0;
        self.credit_max = 0;
        self.backoffs = 0;
        self.backoff_wait_nanos = 0;
        self.packets = 0;
        self.period_nanos = 0;
        self.period_max = 0;
        self.period_count = [0; PERIOD_BUCKETS];
        self.rx_frames = 0;
        self.gap_count = [0; GAP_BUCKETS];
        self.gap_nanos = [0; GAP_BUCKETS];
        self.supply_count = [0; SUPPLY_BUCKETS];
        self.supply_nanos = [0; SUPPLY_BUCKETS];
        self.rdif_at_write = [0; DEPTH_BUCKETS];
        self.queue_at_arrival = [0; DEPTH_BUCKETS];
    }

    fn report(&mut self) {
        let dt_ms = self.elapsed(self.window_open.expect("an open window has a start")) / 1_000_000;
        let steps = OWNER_STEPS.load(Ordering::Relaxed) - self.owner_steps_at_open;
        let calls = OWNER_CALLS.load(Ordering::Relaxed) - self.owner_calls_at_open;
        log::info!(
            "[wifi-probe] pkts={} dt={}ms period_avg={}us max={}us hist={}/{}/{}/{}/{} | write \
             n={} avg={}us max={}us bytes={} | credit n={} min={} max={} avgx10={} backoff={} \
             avg={}us | gap rx0 n={} avg={}us | rx1 n={} avg={}us | rx2+ n={} avg={}us | supply \
             core n={} avg={}us | ring n={} avg={}us | none n={} avg={}us | steps={} \
             per_pkt={}.{} owner_calls={}",
            self.packets,
            dt_ms,
            average_us(self.period_nanos, self.packets),
            self.period_max / 1_000,
            self.period_count[0],
            self.period_count[1],
            self.period_count[2],
            self.period_count[3],
            self.period_count[4],
            self.class_count[TX_WRITE],
            average_us(self.class_nanos[TX_WRITE], self.class_count[TX_WRITE]),
            self.class_max[TX_WRITE] / 1_000,
            self.class_bytes[TX_WRITE],
            self.credit_samples,
            self.credit_min,
            self.credit_max,
            self.credit_sum * 10 / self.credit_samples.max(1),
            self.backoffs,
            average_us(self.backoff_wait_nanos, self.backoffs),
            self.gap_count[0],
            average_us(self.gap_nanos[0], self.gap_count[0]),
            self.gap_count[1],
            average_us(self.gap_nanos[1], self.gap_count[1]),
            self.gap_count[2],
            average_us(self.gap_nanos[2], self.gap_count[2]),
            self.supply_count[SUPPLY_CORE],
            average_us(
                self.supply_nanos[SUPPLY_CORE],
                self.supply_count[SUPPLY_CORE]
            ),
            self.supply_count[SUPPLY_RING],
            average_us(
                self.supply_nanos[SUPPLY_RING],
                self.supply_count[SUPPLY_RING]
            ),
            self.supply_count[SUPPLY_NONE],
            average_us(
                self.supply_nanos[SUPPLY_NONE],
                self.supply_count[SUPPLY_NONE]
            ),
            steps,
            steps / self.packets.max(1),
            steps % self.packets.max(1) * 10 / self.packets.max(1),
            calls,
        );
        log::info!(
            "[wifi-probe-rx] data n={} avg={}us max={}us bytes={} frames={} | size <=512 n={} \
             avg={}us | <=2k n={} avg={}us | <=8k n={} avg={}us | >8k n={} avg={}us | control \
             n={} avg={}us max={}us | credit n={} avg={}us | other n={} avg={}us | rdif_at_write \
             0/1/2-3/4+={}/{}/{}/{} | queue_at_arrival={}/{}/{}/{}",
            self.class_count[RX_DATA],
            average_us(self.class_nanos[RX_DATA], self.class_count[RX_DATA]),
            self.class_max[RX_DATA] / 1_000,
            self.class_bytes[RX_DATA],
            self.rx_frames,
            self.rx_size_count[0],
            average_us(self.rx_size_nanos[0], self.rx_size_count[0]),
            self.rx_size_count[1],
            average_us(self.rx_size_nanos[1], self.rx_size_count[1]),
            self.rx_size_count[2],
            average_us(self.rx_size_nanos[2], self.rx_size_count[2]),
            self.rx_size_count[3],
            average_us(self.rx_size_nanos[3], self.rx_size_count[3]),
            self.class_count[RX_CONTROL],
            average_us(self.class_nanos[RX_CONTROL], self.class_count[RX_CONTROL]),
            self.class_max[RX_CONTROL] / 1_000,
            self.class_count[TX_CREDIT],
            average_us(self.class_nanos[TX_CREDIT], self.class_count[TX_CREDIT]),
            self.class_count[OTHER],
            average_us(self.class_nanos[OTHER], self.class_count[OTHER]),
            self.rdif_at_write[0],
            self.rdif_at_write[1],
            self.rdif_at_write[2],
            self.rdif_at_write[3],
            self.queue_at_arrival[0],
            self.queue_at_arrival[1],
            self.queue_at_arrival[2],
            self.queue_at_arrival[3],
        );
        self.open_window(self.now);
    }
}

const fn classify(purpose: &IoPurpose) -> usize {
    match purpose {
        IoPurpose::TransmitData => TX_WRITE,
        IoPurpose::TransmitFlow => TX_CREDIT,
        IoPurpose::ReceiveData(_) => RX_DATA,
        IoPurpose::ReceiveCount(_)
        | IoPurpose::ReceiveByteLength(_)
        | IoPurpose::ReceiveOtherAck(_)
        | IoPurpose::ReceiveOtherClear(_) => RX_CONTROL,
        IoPurpose::Startup
        | IoPurpose::Shutdown
        | IoPurpose::MailboxFlow
        | IoPurpose::MailboxWrite => OTHER,
    }
}

fn request_bytes(kind: &SdioRequestKind) -> u64 {
    match kind {
        SdioRequestKind::Read { length, .. } => *length as u64,
        SdioRequestKind::Write { bytes, .. } => bytes.len() as u64,
        _ => 1,
    }
}

const fn rx_size_bucket(bytes: u64) -> usize {
    if bytes <= 512 {
        0
    } else if bytes <= 2048 {
        1
    } else if bytes <= 8192 {
        2
    } else {
        3
    }
}

const fn period_bucket(nanos: u64) -> usize {
    if nanos < 250_000 {
        0
    } else if nanos < 600_000 {
        1
    } else if nanos < 1_000_000 {
        2
    } else if nanos < 1_500_000 {
        3
    } else {
        4
    }
}

const fn depth_bucket(depth: usize) -> usize {
    match depth {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        _ => 3,
    }
}

fn average_us(total_nanos: u64, count: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    total_nanos / count / 1_000
}
