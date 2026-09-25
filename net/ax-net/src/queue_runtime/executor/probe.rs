//! Temporary board-measurement probe for the queue-executor loop.
//!
//! Diagnostic only.  The window is reported on the serial console every two
//! seconds, matching the device-side probe window, so both logs can be read
//! side by side: how much wall clock the executor spends polling, how much it
//! spends inside the device owner, and how much it spends asleep waiting for an
//! interrupt.
//!
//! The probe also samples how many frames the protocol had already handed over
//! (`tx_ready`) when the owner was advanced, which is the layer above the
//! device ring the device-side probe samples at the same boundary.
//!
//! Two further splits price the device round trip itself: how long after a
//! device interrupt the owner was advanced again (wakeup latency), and whether
//! a blocking wait ended by notification or by reaching its deadline.

/// Window flush period.
const WINDOW_NANOS: u64 = 2_000_000_000;
/// Owner calls above this are counted as slow, to price per-call work.
const OWNER_SLOW_NANOS: u64 = 200_000;
/// Pending protocol frames: none, one, two to three, four or more.
const DEPTH_BUCKETS: usize = 4;
/// Interrupt-to-owner latency: under 20, 60, 150, 400 us, then more.
const WAKE_BUCKETS: usize = 5;
/// Blocking wait duration: under 50, 150, 250, 500 us, then more.
const WAIT_BUCKETS: usize = 5;

#[derive(Default)]
pub(super) struct ExecutorProbe {
    window_open: Option<u64>,
    iters: u64,
    polls: u64,
    poll_nanos: u64,
    waits: u64,
    wait_nanos: u64,
    wait_notified: u64,
    wait_notified_nanos: u64,
    wait_deadline: u64,
    wait_deadline_nanos: u64,
    wait_hist: [u64; WAIT_BUCKETS],
    yields: u64,
    rearm_calls: u64,
    rearm_skipped: u64,
    owner_nanos: u64,
    owner_max: u64,
    owner_slow: u64,
    wake_count: u64,
    wake_nanos: u64,
    wake_hist: [u64; WAKE_BUCKETS],
    tx_submitted: u64,
    tx_reclaimed: u64,
    rx_reclaimed: u64,
    ready_depth: [u64; DEPTH_BUCKETS],
    irq_at_open: u64,
    irq: u64,
}

impl ExecutorProbe {
    /// Books one executor loop iteration and closes a finished window.
    pub(super) fn tick(&mut self, now_nanos: u64, irq: u64) {
        self.irq = irq;
        match self.window_open {
            None => self.open_window(now_nanos),
            Some(window) if now_nanos.saturating_sub(window) >= WINDOW_NANOS => {
                // An idle window would only repeat the wait it slept in, so it
                // is closed silently.
                if self.polls == 0 && self.rearm_calls == 0 {
                    self.open_window(now_nanos);
                } else {
                    self.report(now_nanos);
                }
            }
            Some(_) => {}
        }
        self.iters += 1;
    }

    pub(super) fn poll(&mut self, start_nanos: u64, end_nanos: u64) {
        self.polls += 1;
        self.poll_nanos += end_nanos.saturating_sub(start_nanos);
    }

    /// Books a blocking wait; `by_deadline` distinguishes a reached deadline
    /// from a notification that arrived first.
    pub(super) fn wait(&mut self, start_nanos: u64, end_nanos: u64, by_deadline: bool) {
        let nanos = end_nanos.saturating_sub(start_nanos);
        self.waits += 1;
        self.wait_nanos += nanos;
        if by_deadline {
            self.wait_deadline += 1;
            self.wait_deadline_nanos += nanos;
        } else {
            self.wait_notified += 1;
            self.wait_notified_nanos += nanos;
        }
        self.wait_hist[wait_bucket(nanos)] += 1;
    }

    /// Books how long after the last device interrupt the owner was advanced.
    /// `last_irq_nanos` is `None` when no interrupt has arrived yet.
    pub(super) fn wake(&mut self, now_nanos: u64, last_irq_nanos: Option<u64>) {
        let Some(irq) = last_irq_nanos else {
            return;
        };
        let nanos = now_nanos.saturating_sub(irq);
        self.wake_count += 1;
        self.wake_nanos += nanos;
        self.wake_hist[wake_bucket(nanos)] += 1;
    }

    pub(super) fn yielded(&mut self) {
        self.yields += 1;
    }

    /// Books one owner advance: its duration and how many protocol frames were
    /// still waiting for it.
    pub(super) fn owner_call(&mut self, start_nanos: u64, end_nanos: u64, ready_depth: usize) {
        let nanos = end_nanos.saturating_sub(start_nanos);
        self.rearm_calls += 1;
        self.owner_nanos += nanos;
        self.owner_max = self.owner_max.max(nanos);
        if nanos > OWNER_SLOW_NANOS {
            self.owner_slow += 1;
        }
        self.ready_depth[depth_bucket(ready_depth)] += 1;
    }

    /// One owner advance was skipped because the group had been rescheduled.
    pub(super) fn owner_skipped(&mut self) {
        self.rearm_skipped += 1;
    }

    pub(super) fn tx_submitted(&mut self) {
        self.tx_submitted += 1;
    }

    pub(super) fn tx_reclaimed(&mut self) {
        self.tx_reclaimed += 1;
    }

    pub(super) fn rx_reclaimed(&mut self) {
        self.rx_reclaimed += 1;
    }

    fn open_window(&mut self, now_nanos: u64) {
        self.window_open = Some(now_nanos);
        self.irq_at_open = self.irq;
        self.iters = 0;
        self.polls = 0;
        self.poll_nanos = 0;
        self.waits = 0;
        self.wait_nanos = 0;
        self.wait_notified = 0;
        self.wait_notified_nanos = 0;
        self.wait_deadline = 0;
        self.wait_deadline_nanos = 0;
        self.wait_hist = [0; WAIT_BUCKETS];
        self.wake_count = 0;
        self.wake_nanos = 0;
        self.wake_hist = [0; WAKE_BUCKETS];
        self.yields = 0;
        self.rearm_calls = 0;
        self.rearm_skipped = 0;
        self.owner_nanos = 0;
        self.owner_max = 0;
        self.owner_slow = 0;
        self.tx_submitted = 0;
        self.tx_reclaimed = 0;
        self.rx_reclaimed = 0;
        self.ready_depth = [0; DEPTH_BUCKETS];
    }

    fn report(&mut self, now_nanos: u64) {
        let dt_ms = now_nanos.saturating_sub(self.window_open.expect("open window")) / 1_000_000;
        log::info!(
            "[netprobe] dt={}ms iters={} polls={} poll_us={} waits={} wait_us={} yields={} | \
             wait_kind notify={}@{}us deadline={}@{}us hist={}/{}/{}/{}/{} | wake n={} avg={}us \
             hist={}/{}/{}/{}/{} | owner_calls={} skipped={} owner_us={} max={}us slow={} | \
             tx_submit={} tx_done={} rx_done={} | ready 0/1/2-3/4+={}/{}/{}/{} | irq={}",
            dt_ms,
            self.iters,
            self.polls,
            average_us(self.poll_nanos, self.polls),
            self.waits,
            average_us(self.wait_nanos, self.waits),
            self.yields,
            self.wait_notified,
            average_us(self.wait_notified_nanos, self.wait_notified),
            self.wait_deadline,
            average_us(self.wait_deadline_nanos, self.wait_deadline),
            self.wait_hist[0],
            self.wait_hist[1],
            self.wait_hist[2],
            self.wait_hist[3],
            self.wait_hist[4],
            self.wake_count,
            average_us(self.wake_nanos, self.wake_count),
            self.wake_hist[0],
            self.wake_hist[1],
            self.wake_hist[2],
            self.wake_hist[3],
            self.wake_hist[4],
            self.rearm_calls,
            self.rearm_skipped,
            average_us(self.owner_nanos, self.rearm_calls),
            self.owner_max / 1_000,
            self.owner_slow,
            self.tx_submitted,
            self.tx_reclaimed,
            self.rx_reclaimed,
            self.ready_depth[0],
            self.ready_depth[1],
            self.ready_depth[2],
            self.ready_depth[3],
            self.irq.saturating_sub(self.irq_at_open),
        );
        self.open_window(now_nanos);
    }
}

const fn wait_bucket(nanos: u64) -> usize {
    if nanos < 50_000 {
        0
    } else if nanos < 150_000 {
        1
    } else if nanos < 250_000 {
        2
    } else if nanos < 500_000 {
        3
    } else {
        4
    }
}

const fn wake_bucket(nanos: u64) -> usize {
    if nanos < 20_000 {
        0
    } else if nanos < 60_000 {
        1
    } else if nanos < 150_000 {
        2
    } else if nanos < 400_000 {
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
