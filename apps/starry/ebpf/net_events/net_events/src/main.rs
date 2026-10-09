//! Attach to the six `net:*` tracepoints and print their readings on demand.
//!
//! The tool never generates traffic.  It attaches once, the counters accumulate
//! in the kernel while the load is driven by hand, and a reading is printed
//! whenever one is asked for.  One shell is enough:
//!
//! ```text
//! net-events &          attach, print reading [1], hand the prompt back
//! iperf3 -c <peer>      the load, run by hand and timed by hand
//! kill -USR1 %1         the next reading: the window since the last one
//! kill %1               detach and exit
//! ```
//!
//! Every print has the same shape, the first one included: reading [1] is the
//! moment of the attach, and each later reading is the window after the one
//! before it.  The kernel's counters are cumulative, so the loader subtracts
//! what it printed last.  A reading counts what the events reported, not what
//! the interface carried — a disabled or dropped record is simply absent.

use std::{
    io::{BufRead as _, BufReader},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use aya::{
    Ebpf, include_bytes_aligned,
    maps::{Array, MapData},
    programs::TracePoint,
};
use net_events_common::*;

/// Where the tracepoints publish their records and their enable state.
const EVENTS_ROOT: &str = "/sys/kernel/debug/tracing/events";

/// A reading was asked for (SIGUSR1, or a line on stdin).
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// The tool was asked to stop (SIGINT and SIGTERM).
static STOPPING: AtomicBool = AtomicBool::new(false);

/// The counter table as the loader holds it: owned, so the map outlives the
/// `Ebpf` object's program lookups.
type CounterTable = Array<MapData, u64>;

fn main() -> anyhow::Result<()> {
    let selected = parse_arguments()?;

    // Refuse to attach to an event whose published record is not the one the
    // offsets in the eBPF program assume: a moved field would otherwise be
    // read as its neighbour and counted as a plausible number.
    for spec in &selected {
        verify_event(spec)?;
    }

    let mut ebpf = Ebpf::load(include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/net_events"
    )))?;

    // The links are held for the whole run: dropping one detaches its program.
    let mut links = Vec::with_capacity(selected.len());
    for spec in &selected {
        let program: &mut TracePoint = ebpf
            .program_mut(spec.name)
            .ok_or_else(|| anyhow!("the object carries no program for net:{}", spec.name))?
            .try_into()?;
        program.load()?;
        // Attaching through the perf event path is what opens the event's
        // gate: no tracefs `enable` write is involved.
        links.push(
            program
                .attach("net", spec.name)
                .with_context(|| format!("cannot attach to net:{}", spec.name))?,
        );
    }
    let _links = links;

    let counters = CounterTable::try_from(
        ebpf.take_map("COUNTERS")
            .ok_or_else(|| anyhow!("the object carries no COUNTERS map"))?,
    )?;

    install_signal_handlers();
    watch_stdin();

    let mut previous = vec![0u64; COUNTER_SLOTS as usize];
    let mut index = 0usize;
    let mut mark = Instant::now();
    loop {
        let stopping = STOPPING.load(Ordering::SeqCst);
        let requested = REQUESTED.swap(false, Ordering::SeqCst);
        // The first pass prints reading [1] without waiting for a trigger, so
        // the attach is announced by a reading and nothing else; every later
        // pass is a trigger.
        if index > 0 && !stopping && !requested {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let current = read_counters(&counters);
        index += 1;
        print_reading(index, mark.elapsed(), &current, &previous, &selected);
        previous = current;
        mark = Instant::now();
        if stopping {
            break;
        }
    }
    Ok(())
}

/// Parse `--events <name>[,<name>...]`, defaulting to every event.
fn parse_arguments() -> anyhow::Result<Vec<&'static EventSpec>> {
    let mut selected: Option<Vec<&'static EventSpec>> = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--events" => {
                let list = arguments
                    .next()
                    .ok_or_else(|| anyhow!("--events needs a comma-separated list"))?;
                let mut chosen = Vec::new();
                for name in list.split(',') {
                    let name = name.trim();
                    if name.is_empty() {
                        continue;
                    }
                    chosen.push(EVENTS.iter().find(|event| event.name == name).ok_or_else(
                        || anyhow!("unknown event '{name}'; known: {}", known_names()),
                    )?);
                }
                selected = Some(chosen);
            }
            "-h" | "--help" => {
                println!("usage: net-events [--events <name>[,<name>...]]");
                println!();
                println!("attaches to the net:* tracepoints and prints a reading on");
                println!("SIGUSR1 (or on a line read from stdin), each one describing");
                println!("the window since the previous reading.");
                println!();
                println!("known events: {}", known_names());
                std::process::exit(0);
            }
            other => bail!("unexpected argument '{other}' (try --help)"),
        }
    }

    let selected = selected.unwrap_or_else(|| EVENTS.iter().collect());
    if selected.is_empty() {
        bail!("no event selected");
    }
    Ok(selected)
}

fn known_names() -> String {
    EVENTS
        .iter()
        .map(|event| event.name)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check the tracepoint exists and declares the fields the offsets assume, in
/// the order they are read.
fn verify_event(spec: &EventSpec) -> anyhow::Result<()> {
    let format_path = format!("{EVENTS_ROOT}/net/{}/format", spec.name);
    let text = std::fs::read_to_string(&format_path)
        .with_context(|| format!("cannot read {format_path}"))?;

    let mut cursor = 0usize;
    for field in spec.fields {
        let Some(at) = text[cursor..].find(field).map(|at| at + cursor) else {
            bail!(
                "net:{} does not declare '{field}' after the fields before it; \
                 the record layout is not the one this tool reads",
                spec.name
            );
        };
        cursor = at + field.len();
    }
    Ok(())
}

fn install_signal_handlers() {
    extern "C" fn on_request(_: libc::c_int) {
        REQUESTED.store(true, Ordering::SeqCst);
    }
    extern "C" fn on_stop(_: libc::c_int) {
        STOPPING.store(true, Ordering::SeqCst);
    }

    // SAFETY: both handlers only store to an atomic flag, which is
    // async-signal-safe, and the signal numbers are valid.
    unsafe {
        libc::signal(libc::SIGUSR1, on_request as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_stop as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_stop as libc::sighandler_t);
        // Started with `&`, this process is not in the terminal's foreground
        // group, and reading the terminal would stop it with SIGTTIN.  Ignoring
        // that (and SIGTTOU) lets the background read fail harmlessly and
        // leaves SIGUSR1 as the trigger that works there.
        libc::signal(libc::SIGTTIN, libc::SIG_IGN);
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
    }
}

/// Let a line on stdin ask for a reading, so the tool can also be watched in
/// the foreground while the load comes from the other end of the link.
fn watch_stdin() {
    thread::spawn(|| {
        let stdin = std::io::stdin();
        for line in BufReader::new(stdin.lock()).lines() {
            if line.is_err() {
                break;
            }
            REQUESTED.store(true, Ordering::SeqCst);
        }
    });
}

/// Sum each slot across the owner CPU regions.
///
/// The table is a plain array, so this reads the same values from whichever CPU
/// the loader runs on — which a per-CPU map would not (see the layout notes in
/// `net_events-common`).
fn read_counters(counters: &CounterTable) -> Vec<u64> {
    (0..COUNTER_SLOTS)
        .map(|slot| {
            (0..PARTITIONS)
                .map(
                    |partition| match counters.get(&(partition * COUNTER_SLOTS + slot), 0) {
                        Ok(value) => value,
                        // An entry that cannot be read is reported as nothing rather
                        // than aborting the run: the rest of the reading is still
                        // worth printing.
                        Err(error) => {
                            eprintln!(
                                "net-events: entry {partition}/{slot} could not be read: {error}"
                            );
                            0
                        }
                    },
                )
                .sum()
        })
        .collect()
}

/// Print one reading: the cumulative total with the window's growth beside it,
/// and the window's own numbers spelled out per event.
///
/// Every reading has this shape; the first one carries no window, because
/// there is nothing before it to measure from.
fn print_reading(
    index: usize,
    elapsed: Duration,
    current: &[u64],
    previous: &[u64],
    events: &[&EventSpec],
) {
    println!();
    if index == 1 {
        println!("[1]");
    } else {
        println!(
            "[{index}] +{:.1}s since [{}]",
            elapsed.as_secs_f64(),
            index - 1
        );
    }

    for spec in events {
        let base = spec.base as usize;
        let total = current[base];
        let grew = total.saturating_sub(previous[base]);
        let window = Window {
            current: &current[base..],
            previous: &previous[base..],
        };
        let detail = if grew == 0 {
            match spec.kind {
                EventKind::Rearm => "not driven in this window (needs a race at rearm)".to_string(),
                EventKind::Backpressure => {
                    "not driven in this window (needs a busy device)".to_string()
                }
                _ => "not seen in this window".to_string(),
            }
        } else {
            describe(spec.kind, &window)
        };
        println!("  {:<18} {total:>12} {grew:>+12}   {detail}", spec.name);
    }
}

/// One event's window: its block of the table, current and previous.
struct Window<'a> {
    current: &'a [u64],
    previous: &'a [u64],
}

impl Window<'_> {
    /// How much one slot of the block grew in this window.
    fn grew(&self, slot: u32) -> u64 {
        self.current[slot as usize].saturating_sub(self.previous[slot as usize])
    }
}

/// Spell out one event's window in the event's own terms.
fn describe(kind: EventKind, window: &Window<'_>) -> String {
    match kind {
        EventKind::Poll => {
            let rounds = window.grew(TOTAL);
            let units = window.grew(POLL_WORK_UNITS);
            let per_round = if rounds == 0 {
                0.0
            } else {
                units as f64 / rounds as f64
            };
            format!(
                "idle={} more={} blocked={} failed={}   work_units={units} ({per_round:.1}/round)",
                window.grew(POLL_IDLE),
                window.grew(POLL_MORE),
                window.grew(POLL_BLOCKED),
                window.grew(POLL_FAILED),
            )
        }
        EventKind::Rearm => format!(
            "race={} work_pending={} retry_at={} failed={}",
            window.grew(REARM_RACE),
            window.grew(REARM_WORK_PENDING),
            window.grew(REARM_RETRY_AT),
            window.grew(REARM_FAILED),
        ),
        EventKind::Backpressure => format!(
            "tx/retry={} tx/link_down={} rx/refill_retry={}",
            window.grew(BACKPRESSURE_TX_RETRY),
            window.grew(BACKPRESSURE_TX_LINK_DOWN),
            window.grew(BACKPRESSURE_RX_RETRY),
        ),
        EventKind::TxSubmit | EventKind::RxPublish => {
            let frames = window.grew(TOTAL);
            let bytes = window.grew(FRAME_BYTES);
            let average = if frames == 0 {
                0.0
            } else {
                bytes as f64 / frames as f64
            };
            format!("frames={frames} bytes={bytes} (avg {average:.0} B/frame)")
        }
        EventKind::ProtoYield => format!(
            "poll_count={} deadline={} both={}   work_pending={}",
            window.grew(YIELD_POLL_COUNT),
            window.grew(YIELD_DEADLINE),
            window.grew(YIELD_BOTH),
            window.grew(YIELD_WORK_PENDING),
        ),
    }
}
