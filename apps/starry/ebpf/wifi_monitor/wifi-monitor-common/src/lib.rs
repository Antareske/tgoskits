#![no_std]

//! eBPF 侧与 loader 共享的常量。

/// 延迟直方图分桶的微秒上界（不含），一共 8 桶。
pub const BUCKET_US_LIMITS: [u64; 8] = [
    300, 500, 1_000, 5_000, 20_000, 50_000, 100_000, u64::MAX,
];

pub const BUCKET_COUNT: usize = BUCKET_US_LIMITS.len();

// ── Map key ──

pub const TX_KEY_CNT: u32 = 0;
pub const TX_KEY_BYTES: u32 = 1;

pub const SDIO_KEY_WR: u32 = 0;

pub const SDIO_ERR_KEY_WR: u32 = 0;

pub const ENTRY_KEY_TS: u32 = 0;
pub const ENTRY_KEY_LEN: u32 = 1;
