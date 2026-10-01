//! Host-side reproduction of the in-kernel kallsyms name lookup.
//!
//! Loads the `.kallsyms` blob extracted from the built StarryOS kernel and
//! replays the exact `KallsymsMapped::lookup_name` calls the kprobe attach
//! path makes, so a hang or mismatch can be diagnosed without a QEMU cycle.

use std::alloc::{Layout, alloc};
use std::io::Read;

const STEXT: u64 = 0xffff_ffff_8000_1000;
const ETEXT: u64 = 0xffff_ffff_8059_d000;

/// The symbols the netmon loader resolves, in its own probe order.
const PROBES: &[(&str, &str)] = &[
    (
        "sched_irq",
        "_RNvMs_NtNtCsfywRPgDWoTN_6ax_net13queue_runtime5stateNtB4_14PollGroupState12schedule_irq",
    ),
    (
        "queue_poll",
        "_RNvMs3_NtNtCsfywRPgDWoTN_6ax_net13queue_runtime8executorNtB5_18QueueGroupExecutor4poll",
    ),
    (
        "port_rx",
        "_RNvXs1_NtNtCsfywRPgDWoTN_6ax_net13queue_runtime8executorNtB5_14QueueFramePortNtNtNtB9_6device6driver17EthernetFramePort7receive",
    ),
    (
        "count_tx (L3, removed)",
        "_RNvMs0_NtCsfywRPgDWoTN_6ax_net6routerNtB5_12DeviceHandle8count_tx",
    ),
];

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "kallsyms.bin".to_string());
    let mut raw = Vec::new();
    std::fs::File::open(&path)
        .expect("open kallsyms.bin")
        .read_to_end(&mut raw)
        .expect("read kallsyms.bin");

    let total = u64::from_le_bytes(raw[0..8].try_into().unwrap()) as usize;
    let num_syms = u64::from_le_bytes(raw[8..16].try_into().unwrap());
    println!(
        "blob: total_bytes={total} section_len={} num_syms={num_syms}",
        raw.len()
    );
    assert!(total <= raw.len(), "total_bytes exceeds the section");

    // `from_blob` aligns the inner arrays against the absolute buffer address,
    // so the slice must be page aligned exactly as the linker section is.
    let layout = Layout::from_size_align(total, 4096).expect("layout");
    let aligned = unsafe { alloc(layout) };
    assert!(!aligned.is_null(), "allocation failed");
    unsafe { std::ptr::copy_nonoverlapping(raw.as_ptr(), aligned, total) };
    let blob = unsafe { std::slice::from_raw_parts(aligned, total) };

    let mapped = ksym::KallsymsMapped::from_blob(blob, STEXT, ETEXT).expect("from_blob");
    println!("from_blob OK");

    for (label, name) in PROBES {
        println!("---- lookup_name [{label}] len={}", name.len());
        match mapped.lookup_name(name) {
            Some(addr) => println!("     -> {addr:#x}"),
            None => println!("     -> None (symbol absent)"),
        }
    }

    // Round-trip every symbol in the table: the name the table reports for an
    // address must resolve back to that address.
    let dump = mapped.dump_all_symbols();
    let mut checked = 0usize;
    let mut mismatched = 0usize;
    for line in dump.lines() {
        let mut parts = line.splitn(3, ' ');
        let Some(addr_hex) = parts.next() else { continue };
        let Some(_ty) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        if name.is_empty() {
            continue;
        }
        let addr = u64::from_str_radix(addr_hex, 16).expect("dump address");
        checked += 1;
        if checked % 1000 == 0 {
            println!("     round-trip progress: {checked}");
        }
        match mapped.lookup_name(name) {
            Some(found) if found == addr => {}
            Some(found) => {
                mismatched += 1;
                if mismatched <= 10 {
                    println!("MISMATCH {name}: dump={addr:#x} lookup={found:#x}");
                }
            }
            None => {
                mismatched += 1;
                if mismatched <= 10 {
                    println!("NOT FOUND by lookup_name: {name} (dump addr {addr:#x})");
                }
            }
        }
    }
    println!("round-trip done: checked={checked} mismatched={mismatched}");
}
