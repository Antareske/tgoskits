# SG2002 WiFi TX MMIO 时序发现 — 2026-08-05T18:00

> 实验揭示了 `poll_int_status` Phase 1 自旋轮询在缺少硬件 fence 时不可靠，
> 印证了之前 0.85 Mbps 测试中 `delay_ms(10)` 频繁被触发的推测。

## 背景

`sg2002/wifi-irq` 分支将 `poll_int_status` 改为两阶段：

- **Phase 1**：1000 次 MMIO 读自旋（~50µs），覆盖绝大多数硬件响应
- **Phase 2**：XFER_COMPLETE 走中断唤醒，其余位 `delay_ms(10)` × 20 次

之前 0.85 Mbps 的 TX 测试怀疑非 XFER 位的 10ms 睡眠是瓶颈，但没有板上证据。

## 实验设计

四组对照镜像，除内核代码外所有资产（fip、ramdisk、DTB、rootfs）完全相同：

| 镜像 | 内核 | 特征 |
|------|------|------|
| wifiirq | `sg2002/wifi-irq` 最新 (4f76933fb) | XFER 中断 + **诊断计数器（含 now_nanos() 调用）** |
| nodiag | `5257f217c` | XFER 中断，**无 now_nanos()** |
| dev | tgoskits `dev` | 无 XFER 中断改动 |
| devfix | `dev` + wifi fix (0dce0edc) | 无 XFER 中断改动 |

## 实验数据

### Image 1 (wifiirq, 有 now_nanos()) — irq-diag.log

```
TX: 11.0 Mbps, 稳定, Retr=0
RX: 12.3 Mbps, 稳定
```

### Image 4 (nodiag, 无 now_nanos()) — irq-nodiag.log

```
TX: 1.16 Mbps, 128KB burst-gap, Cwnd=0
RX: 12.1 Mbps, 稳定
```

TX 吞吐差约 **9.5 倍**（11.0 vs 1.16），nodiag 回到了和之前 0.85 Mbps 相同的 burst-gap 模式。

## 根因分析

Image 1 和 Image 4 内核的唯一区别是诊断计数器代码：

**Image 1（有诊断）**——`poll_int_status` 入口处：

```rust
let t_entry = crate::runtime::delay().now_nanos();
// 这会调用 ax_hal::time::monotonic_time_nanos() → 读 RISC-V mtime CSR
// ↓ 之后进入 Phase 1 自旋
for _ in 0..PHASE1_SPIN_ITERS { ... }
```

**Image 4（无诊断）**——`poll_int_status` 入口处：

```rust
// 没有 now_nanos() 调用
// ↓ 直接进入 Phase 1 自旋
for _ in 0..PHASE1_SPIN_ITERS { ... }
```

`now_nanos()` → `monotonic_time_nanos()` 读 RISC-V `mtime` CSR（映射到 MMIO 地址空间的硬件定时器）。**这个 MMIO 读充当了硬件 fence**：

1. **编译器层面**：`read_volatile` 阻止编译器将 Phase 1 循环中的 `poll_status_once` MMIO 读重排到 `now_nanos()` 之前
2. **CPU 层面**：RISC-V MMIO 访问需要通过内存系统，读 `mtime` 寄存器会刷新 load/store 队列，确保之前的操作完成、后续的 MMIO 读不会在总线层面被合并或重排

没有这个 fence 时，Phase 1 的 `poll_status_once` 循环可能在硬件状态位被置位后仍然读到旧值——因为 CPU 的 store buffer 中的数据尚未完全刷新到 SDHCI 寄存器的总线域。1000 次自旋（~50µs）刚好覆盖正常硬件延迟，但如果 MMIO 读在总线层面有延迟或重排，实际"看到"新值的时间窗口就会缩小。

**后果**：Phase 1 的 50µs 窗口对漏读敏感——只要 256 blocks × 每次 BUF_WR_READY 有一个掉入 Phase 2，就是 10ms 白等。累积起来，一次 128KB TX 就会多出几十到上百毫秒延迟，表现为 burst-gap。

## 推论

1. **Phase 1 的 50µs 窗口本身是够的**——加上 fence 后 Phase 1 命中率恢复正常，TX 达到 11 Mbps
2. **10ms 睡眠不是架构问题，是 fence 缺失的放大效应**——1000 次自旋如果没有可靠的 MMIO 语义，等于白转
3. **之前的 `yield_now()` 自旋方案**（Part 1 的 ~10 Mbps）之所以工作，可能是因为 `yield_now()` 内部的调度器操作（任务切换、上下文保存）隐含了足够的 fence 效果
4. **中断驱动的 `block_timeout` 路径**（XFER_COMPLETE）不受影响，因为 ISR 本身就是强 fence 事件

## 修复方向

不需要退回忙等，而是在 Phase 1 循环的每次迭代中确保 MMIO 读有 fence 语义：

- 当前 `core::hint::spin_loop()` 在 RISC-V 上只生成 `pause` 指令（等价于 `nop`），不含 fence
- 简单修复：在 `poll_status_once` 读 `INT_STATUS_NORM` 之前或之后加一条 `fence iorw, iorw` 指令，或直接移除 `spin_loop()` 让连续的 MMIO 读自然产生依赖链
- 或者：保留 `now_nanos()` 的 fence 效应，在 Phase 1 入口加一次显式的硬件 fence（`core::sync::atomic::fence(Ordering::SeqCst)` 或 RISC-V `fence` 指令）

## 影响范围

此发现也解释了为什么 dev 主线（无 XFER 中断改动）和 devfix 的 TX 也能达到 ~10 Mbps：它们使用的是旧的 `yield_now()` 自旋方案，每次 `yield_now()` 穿越调度器，调度器的上下文切换指令序列（保存/恢复寄存器、操作 run_queue）隐含了 fence。中断驱动方案消除了 yield，但也消除了隐含的 fence——这是一个意外的退步。

## 实验文件

- `irq-diag.log` — Image 1 (wifiirq, 有 now_nanos)
- `irq-nodiag.log` — Image 4 (nodiag, 无 now_nanos)
- `1.log` — 首次测试（Image 1 构建后，RX 11.3 / TX 10.5）
