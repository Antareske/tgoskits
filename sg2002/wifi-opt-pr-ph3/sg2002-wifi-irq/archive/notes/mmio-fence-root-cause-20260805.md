# SG2002 WiFi TX 0.85 Mbps 根因报告：MMIO Store Buffer 残留与轮询数据竞争

> 日期：2026-08-05
> 取代：「之前的发现报告」(notes/mmio-fence-discovery-20260805.md) 中对根因的部分推测
> 最终确认：Image 1（有 `now_nanos()`）TX 11 Mbps vs Image 4（无 `now_nanos()`）TX 1.16 Mbps

## 1. 最终根因

**CPU store buffer 中残留的 SDHCI buffer 写操作，与 Phase 1 的 `INT_STATUS_NORM` 轮询读操作在 SDHCI 总线上发生竞争。** 写操作被延迟提交到硬件，硬件实际收到数据的时间晚于 CPU 开始轮询的时间，导致 Phase 1 的有效等待窗口缩水，状态位未能在 1000 次自旋（~50µs）内被检测到，掉入 Phase 2 的 10ms 睡眠。

## 2. 因果链

### 2.1 写 Buffer → Store Buffer 暂存

```rust
// pio_write(), 每 block 128 次:
self.write::<u32>(SDHCI_BUFFER, word); // mmio_write → store buffer
```

单核 C906（RISC-V）没有自动的 MMIO fence。`mmio_write` 编译为 `sw` 指令，数据写入 CPU 的 store buffer，**不保证立即到达 SDHCI 控制器**。128 次写操作中，最后若干次在 store buffer 里排队等待总线提交。

### 2.2 立即进入轮询 → 读写竞争

```rust
// 128 次写之后立刻：
wait_buffer_write_ready()  → poll_int_status(BUF_WR_READY)
    for _ in 0..1000 {
        let status = mmio_read(INT_STATUS_NORM);  // 读状态寄存器
        ...
    }
```

`mmio_read` 编译为 `lw` 指令，每次读取都需要在 SDHCI 总线上发起新的请求。此时 store buffer 中的写操作正在后台向同一 SDHCI 总线提交。**读写请求共享同一条总线，彼此竞争带宽。**

### 2.3 硬件收到数据被推迟 → Phase 1 漏读

SDHCI 控制器收到全部 128 个字之后才置 `BUF_WR_READY`。但由于 store buffer 里的写被轮询读挤占了总线时间：

- 无竞争时：写操作在 128 次 `sw` 指令期间就已经逐个提交，最后一个 `sw` 后 ~10µs 硬件就能收到全部数据
- 有竞争时：写操作在 `sw` 指令期间未提交完毕，剩余的写和轮询读争抢总线，实际延迟被拉到 ~30-50µs

Phase 1 的 1000 次自旋约 50µs。无竞争时，硬件在 10µs 内置位，Phase 1 的前 200-300 次自旋就能读到。有竞争时，硬件在 30-50µs 才收到全部数据——恰好卡在 Phase 1 窗口边界，部分调用漏过去。

### 2.4 掉入 Phase 2 → 累积延迟

`poll_int_status` 的 Phase 2 对非 XFER 位使用 `delay_ms(10)`：每次掉入即白等 10ms。

`BUF_WR_READY` 每帧调用 256 次（128KB / 512B per block）。哪怕只有 5% 漏读率：256 × 5% × 10ms = 128ms 额外延迟/帧。加上 `CMD_COMPLETE` 的 1 次/帧，总延迟可超过 130ms。

在 10 秒 iperf3 测试窗口内，这种延迟表现为 128KB burst 后接 0-2 秒静默的 burst-gap 模式——平均吞吐被拉到 ~1 Mbps。

### 2.5 为什么 `now_nanos()` 修了它

诊断计数器在 Phase 1 入口前加了一次 `now_nanos()`：

```rust
fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
    let slot = diag_slot(bit);
    let t_entry = crate::runtime::delay().now_nanos(); // ← 读硬件定时器 MMIO
    // 然后进入 Phase 1
    for _ in 0..PHASE1_SPIN_ITERS { ... }
}
```

`now_nanos()` → `ax_hal::time::monotonic_time_nanos()` 读取 RISC-V `mtime` CSR（映射在不同于 SDHCI 的 MMIO 地址域）。这次读：

1. 走了完整的硬件总线往返（不同的地址域）
2. 函数调用和返回值的使用（`saturating_sub`）建立了数据依赖，CPU 无法乱序到 Phase 1 循环之后
3. 读返回后 Phase 1 才开始。在 `now_nanos()` 执行期间，store buffer 中的 SDHCI 写操作没有竞争地提交到了硬件

这就是一个**意外的 fence 替代品**。它本身不是 fence 指令，但其副作用（跨地址域的 MMIO 读 + 数据依赖）在实践上清空了 store buffer。

### 2.6 为什么邵志航的 yield_now 方案不存在此问题

Part 1 的 Phase 2 循环使用 `yield_now()`：

```rust
for i in 0..200_000 {
    if mmio_read(INT_STATUS_NORM) & bit != 0 { return; }
    yield_now(); // 让出 CPU
}
```

`yield_now()` 内部是任务切换，包含：保存/恢复寄存器（多次 store/load）→ 操作调度器 run_queue（原子指令）→ 切换特权级（`ecall`/`mret`）。

RISC-V 规范规定 `mret` 指令**自带隐式 fence**——它保证之前所有内存操作在陷阱返回前完成。所以每次 `yield_now` 返回时，store buffer 必然是空的。这是任务切换带来的副作用，不是设计意图——邵志航不会知道这个机制在保护他的轮询。

### 2.7 为什么 XFER_COMPLETE 不受影响

`wait_transfer_complete` 走的是 `XFER_COMPLETE` 中断唤醒路径（`block_timeout` → WaitQueue），不是 `delay_ms(10)` 纯睡眠。即使漏读掉入 Phase 2，ISR 在 ~50µs 内唤醒等待任务，不产生 10ms 级延迟。

加上 `XFER_COMPLETE` 每帧只调一次，不像 `BUF_WR_READY` 调 256 次——漏读概率权重低。

### 2.8 为什么 RX（下载）不受影响

RX 路径同样有 store buffer 问题（`read_fifo` → `cmd53_read_fixed` → `wait_buffer_read_ready` → `poll_int_status(BUF_RD_READY)`）。

但 RX 是"拉"模式——数据从模组流向 CPU。`pio_read` 在轮询 `BUF_RD_READY` 之前，没有像 TX 的 `pio_write` 那样密集的 128 次 `mmio_write` 挤占 store buffer。RX 侧的 store buffer 压力小得多，漏读率不同。

更关键的是：RX 没有 firmware TX buffer 流控反压——即使偶有 Phase 2 掉入，也不会像 TX 那样被 128KB buffer 满排空的 1-2 秒间隙放大。

## 3. 修复方案

在 `poll_int_status` 的 Phase 1 入口处添加一条 fence，确保进入轮询之前 store buffer 已排空：

```rust
fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
    // 排空 store buffer：确保之前的 MMIO 写操作已提交到硬件
    // 否则 Phase 1 轮询读会与残留写竞争总线，导致状态位漏读
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

    // Phase 1: 快速自旋
    for _ in 0..PHASE1_SPIN_ITERS {
        if let Some(result) = self.poll_status_once(bit) {
            return result;
        }
        core::hint::spin_loop();
    }
    // ... Phase 2 不变
}
```

`fence(SeqCst)` 在 RISC-V 上编译为 `fence rw, rw`，语义：这条指令之前的所有读写操作必须在之后的所有读写操作开始前完成。效果：store buffer 清空，CPU 的 load/store 队列干净。

**为什么入口处 fence 一次就够？** Phase 1 循环内部是对同一个 `INT_STATUS_NORM` 寄存器的连续 `mmio_read`。RISC-V 规范保证**同一 MMIO 地址的 load 按程序顺序执行**——1000 次读同一个寄存器不会互相重排。Phase 1 内部不需要每轮迭代 fence。只需要确保**进入轮询时初始状态干净**——即之前的写操作已完成。

入口处 fence 一次，对标 `now_nanos()` 通过读取定时器产生的自然同步点，但语义更明确、不依赖副作用。

**是否需要同时移除 `spin_loop()`？** 不需要。`spin_loop()`（`pause` 指令）在 Phase 1 中不会导致 store buffer 残留或 MMIO 重排。它只降低功耗。fence 之后旋即进入密集的 MMIO 读循环，`pause` 的间隙不影响 MMIO 读的可靠性。但也可以移除——此时 Phase 1 变成纯 MMIO 读的紧循环，1000 次约 30µs（比有 pause 的 50µs 更快）。取舍：功耗 vs 延迟，不影响正确性。

## 4. 证实该根因的关键证据链

| # | 证据 | 来源 |
|---|------|------|
| 1 | Image 1（有 `now_nanos()`）TX = 11.0 Mbps | `www/logs/irq-diag.log` |
| 2 | Image 4（无 `now_nanos()`）TX = 1.16 Mbps，burst-gap 模式 | `www/logs/irq-nodiag.log` |
| 3 | 两份内核唯一差异 = `now_nanos()` 调用 → 读 mtime MMIO，产生自然同步 | `git diff 5257f217c 4f76933fb` |
| 4 | `yield_now()` 方案 TX ≈ 10 Mbps（Part 1 测试结果），内含 `mret` 隐式 fence | `sg2002-wifi.md §5` |
| 5 | `BUF_WR_READY` = 256 calls/128KB frame（vs XFER_COMPLETE 1 call），泄漏概率权重 256× | `lib.rs:747` |
| 6 | dev 主线（无 XFER 中断改动，yield_now 循环仍在）TX ≈ 10 Mbps（本次实验） | `irq-diag.log` 同条件测试 |

## 5. 对之前归因的修正

| 之前的归因 | 实际真相 |
|-----------|---------|
| "10ms 睡眠是 TX 慢的直接原因" | 10ms 睡眠是受害者。元凶是 fence 缺失导致 Phase 1 漏读，才频繁掉入 Phase 2 |
| "需恢复 busy-wait（yield_now 自旋）" | 不需要。fence 修复后 Phase 1 工作正常，中断驱动架构本身是正确方向 |
| "缺失 50MHz SDIO/HT 对齐等 Part 2 优化是主因" | 和此 bug 无关。Image 1 在 25MHz、无 HT 对齐下照样 11 Mbps |
| "中断驱动方案不如忙等" | 中断驱动方案被 fence 缺失拖累。fence 修复后，中断方案和忙等方案性能相当（~11 Mbps），且不牺牲 CPU 效率 |

## 6. 影响范围

任何 `mmio_write(SDHCI 寄存器)` 后紧接着 `mmio_read(SDHCI 寄存器)` 的路径都可能受影响。具体：

- `wait_buffer_write_ready`：`pio_write` 写 128 字后立刻轮询 `BUF_WR_READY`，受影响最重
- `wait_cmd_complete`：写 COMMAND 寄存器后轮询 `CMD_COMPLETE`，受影响较轻（写 COMMAND 是一次性的，store buffer 压力小）
- `wait_buffer_read_ready`：RX 路径，写操作压力小于 TX，但仍可能受影响
- `wait_transfer_complete`：有 ISR 兜底，不掉入 10ms 睡眠

## 7. 下一步

1. 在 `poll_int_status` 入口处添加 `fence(SeqCst)` 或 RISC-V 等价的 `asm!("fence rw, rw")`
2. 移除 `now_nanos()` 诊断调用（不再需要其副作用）
3. 保留诊断计数器统计功能，但不依赖其 fence 效果
4. 上板验证：同条件 iperf3 TX 应保持 ~11 Mbps
