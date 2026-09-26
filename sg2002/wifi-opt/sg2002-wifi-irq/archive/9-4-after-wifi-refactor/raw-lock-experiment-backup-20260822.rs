//! 实验性原始自旋锁: 恢复变基前 `ax_kspin::SpinRaw` 语义。
//!
//! 变基把本驱动所有锁从 `ax_kspin::SpinRaw`(纯自旋, 不改变执行上下文)
//! 换成了 `ax_sync::SpinLock`(其 `lock()` 会禁用内核抢占)。这改变了单核
//! 上 wifi-tx/wifi-rx 线程与用户态任务之间的抢占交错: 实测单向 iperf3
//! 吞吐下降约 20%(下行被量化为 ~1.00 MB/s 步进), 且首次双向测试出现
//! `data flow ctrl timeout` 告警。本模块用 `lock_raw()` 包回 SpinRaw 的
//! 语义, 作为对照实验。
//!
//! # Safety
//!
//! `SpinLock::lock_raw` 不提供任何上下文保护, 要求调用方自行排除同核
//! 重入并保证互斥所有权。本驱动的用法满足该契约:
//! - 单核运行(wifi glue 亦 assert `cpu_num == 1`);
//! - ISR/卡中断路径不获取这些锁(见 `sdio1_irq_handler`: 仅 Atomic 标志
//!   + `mask_card_irq` + waker 唤醒);
//! - 不存在同锁递归获取;
//! - 锁内阻塞点(SDIO 传输完成等待)依赖的正是"可被抢占"这一性质——
//!   持有者阻塞时其它任务必须仍可运行, 因此不能使用禁用抢占的
//!   `SpinLock::lock()`。

use ax_sync::{RawSpinLockGuard, SpinLock};

/// 纯自旋互斥锁, 获取/释放不改变执行上下文(等价于旧的 `ax_kspin::SpinRaw`)。
pub struct RawSpinLock<T: ?Sized>(SpinLock<T>);

impl<T> RawSpinLock<T> {
    pub const fn new(data: T) -> Self {
        Self(SpinLock::new(data))
    }
}

impl<T: ?Sized> RawSpinLock<T> {
    pub fn lock(&self) -> RawSpinLockGuard<'_, T> {
        // SAFETY: 见模块文档, 与变基前 ax_kspin::SpinRaw 的契约一致。
        unsafe { self.0.lock_raw() }
    }
}
