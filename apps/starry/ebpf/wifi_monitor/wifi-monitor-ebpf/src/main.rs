#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_ktime_get_ns, bpf_probe_read_kernel},
    macros::{kprobe, kretprobe, map},
    maps::HashMap,
    programs::{ProbeContext, RetProbeContext},
};

// ── TX 帧计数 ──
// key: 0=入队次数, 1=以太网字节累计
#[map]
static TX_FRAME: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(8, 0);

// ── SDIO 写传输字节 ──
#[map]
static SDIO_BYTES: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(8, 0);

// ── SDIO 写错误 ──
// key: 0=write_fifo 失败
#[map]
static SDIO_ERR: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(4, 0);

// ── 写延迟直方图 ──
#[map]
static SDIO_WR_LATENCY: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(16, 0);

// ── write_fifo 入口时间戳暂存 ──
// key: 0=时间戳(ns), 1=字节长度
//
// TODO: 当前为单槽, 而 write_fifo 有多线程调用方 (TX/WPA2 握手/启动),
// 所以并发交错时 entry 可能被覆写
#[map]
static WR_ENTRY: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(4, 0);

// ── 辅助 ──

fn latency_bucket(ns: u64) -> u32 {
    let us = ns / 1_000;
    if us < 300 { 0 }
    else if us < 500 { 1 }
    else if us < 1_000 { 2 }
    else if us < 5_000 { 3 }
    else if us < 20_000 { 4 }
    else if us < 50_000 { 5 }
    else if us < 100_000 { 6 }
    else { 7 }
}

#[inline(always)]
fn inc_map(map: &HashMap<u32, u64>, key: u32) {
    let next = unsafe { map.get(key) }.map(|v| *v + 1).unwrap_or(1);
    let _ = map.insert(key, next, 0);
}

#[inline(always)]
fn add_map(map: &HashMap<u32, u64>, key: u32, delta: u64) {
    let next = unsafe { map.get(key) }
        .map(|v| *v + delta)
        .unwrap_or(delta);
    let _ = map.insert(key, next, 0);
}

// ── 探针 1: enqueue_data_frame ──
// 位于：tx.rs:548  pub fn enqueue_data_frame(bus: &Arc<WifiBus>, eth_frame: Vec<u8>)

#[kprobe]
pub fn tx_enqueue(ctx: ProbeContext) -> u32 {
    try_tx_enqueue(&ctx).unwrap_or(0)
}

fn try_tx_enqueue(ctx: &ProbeContext) -> Result<u32, u32> {
    let vp = ctx.arg::<usize>(1).ok_or(0u32)?;
    let frame_len: u64 = unsafe { bpf_probe_read_kernel((vp + 16) as *const u64) }.unwrap_or(0);
    if frame_len > 0 && frame_len <= 65536 {
        inc_map(&TX_FRAME, 0);
        add_map(&TX_FRAME, 1, frame_len);
    } else if frame_len > 0 {
        inc_map(&TX_FRAME, 0);
    }
    Ok(0)
}

// ── 探针 2: write_fifo ──
// 位于：sdio_transport.rs:166  pub fn write_fifo(&self, func: u8, addr: u32, buf: &[u8])

#[kprobe]
pub fn sdio_write_entry(ctx: ProbeContext) -> u32 {
    try_sdio_write_entry(&ctx).unwrap_or(0)
}

fn try_sdio_write_entry(ctx: &ProbeContext) -> Result<u32, u32> {
    let ts = unsafe { bpf_ktime_get_ns() };
    let len = ctx.arg::<usize>(4).ok_or(0u32)? as u64;
    let _ = WR_ENTRY.insert(0, ts, 0);
    let _ = WR_ENTRY.insert(1, len, 0);
    Ok(0)
}

#[kretprobe]
pub fn sdio_write_return(ctx: RetProbeContext) -> u32 {
    try_sdio_write_return(&ctx).unwrap_or(0)
}

fn try_sdio_write_return(_ctx: &RetProbeContext) -> Result<u32, u32> {
    let now = unsafe { bpf_ktime_get_ns() };
    let entry_ts = unsafe { WR_ENTRY.get(0) }.copied().unwrap_or(0);
    let len = unsafe { WR_ENTRY.get(1) }.copied().unwrap_or(0);
    if entry_ts == 0 {
        return Ok(0);
    }
    let _ = WR_ENTRY.insert(0, 0, 0);
    let _ = WR_ENTRY.insert(1, 0, 0);

    if _ctx.ret::<u64>() != 0 {
        inc_map(&SDIO_ERR, 0);
    }
    let delta = now.saturating_sub(entry_ts);
    inc_map(&SDIO_WR_LATENCY, latency_bucket(delta));
    add_map(&SDIO_BYTES, 0, len);
    Ok(0)
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
