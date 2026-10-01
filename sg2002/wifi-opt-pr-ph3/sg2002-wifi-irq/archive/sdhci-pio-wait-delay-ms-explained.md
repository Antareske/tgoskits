# SDHCI PIO 等待路径：`delay_ms(10)` 为什么是瓶颈，以及中断需求分析

## 前置知识

### SDHCI 的四种标准中断状态位

SDHCI 规范定义了 Normal Interrupt Status Register（偏移 0x30），其中与 PIO 数据传输相关的四个状态位：

| 位 | 名称 | 含义 | SDHCI 标准行为 |
|----|------|------|---------------|
| bit 0 | CMD_COMPLETE | 命令已完成 | 控制器完成一条命令（CMD）的处理后硬件置 1 |
| bit 1 | XFER_COMPLETE | 传输已完成 | 整个数据传输（所有 block）完成后硬件置 1 |
| bit 4 | BUF_WR_READY | 写缓冲区就绪 | SDHCI 内部 FIFO 有空位，可写入下一个 block 的数据 |
| bit 5 | BUF_RD_READY | 读缓冲区就绪 | SDHCI 内部 FIFO 有数据，可读出下一个 block 的数据 |

此外还有：
- bit 8: CARD_INT — SDIO 卡中断（WiFi 芯片通知 host 有数据可读）
- bit 15: ERROR — 错误汇总位（对应 Error Interrupt Status Register）

### PIO 与 DMA 的区别

- **DMA（ADMA2/SDMA）**：Host 控制器自行将数据搬入/搬出内存，CPU 只需等待 `XFER_COMPLETE` 中断即可。所有 block 的 `BUF_WR_READY`/`BUF_RD_READY` 由硬件自动处理。
- **PIO**：CPU 必须逐 32-bit word 将数据写入 `SDHCI_BUFFER`（偏移 0x20）。每写完一个 block（512B），必须等 `BUF_WR_READY` 硬件置位才能写下一个 block。CPU 是搬运工。

SG2002 的 WiFi SDIO 当前使用 PIO 模式——DMA 路径（`block_path.rs` 中的 `submit_write_adma2`）仅用于块设备。

## Starry 的 PIO 写路径：一次 CMD53 写发生了什么

以 TX 一帧 WiFi 数据（~1500B = 3 × 512B blocks）为例，调用链如下：

```
SdioTransport::write_fifo(func=1, addr=WR_FIFO_ADDR, buf)   // sdio_transport.rs:166
  └─ CviSdhci::cmd53_write_fixed()                            // lib.rs:468
       ├─ cmd53_xfer()                                        // lib.rs:383
       │    ├─ wait_data_idle()          ← 自旋等 CMD+DAT 空闲
       │    ├─ 写 SDHCI 寄存器 (BLOCK_SIZE, BLOCK_COUNT, ARGUMENT, TRANSFER_MODE+CMD)
       │    └─ wait_cmd_complete()       ← ★ 等 CMD_COMPLETE (bit 0)
       ├─ pio_write(buf, 512, 3)                              // lib.rs:503
       │    └─ loop 3 blocks:
       │         ├─ wait_buffer_write_ready()  ← ★ 等 BUF_WR_READY (bit 4)
       │         └─ for 128 words: write::<u32>(SDHCI_BUFFER, word)
       └─ wait_transfer_complete()    ← ✓ 等 XFER_COMPLETE (bit 1)
```

每次 CMD53 写需要等待 **4 个不同的条件**满足：

| 等待函数 | 状态位 | 位置 | 每帧触发次数 |
|---------|-------|------|------------|
| `wait_data_idle()` | PRESENT_STATE (非中断位) | `cmd53_xfer()` | 1 |
| `wait_cmd_complete()` | CMD_COMPLETE (bit 0) | `cmd53_xfer()` | 1 |
| `wait_buffer_write_ready()` | BUF_WR_READY (bit 4) | `pio_write()` 每 block 一次 | 3（= block 数） |
| `wait_transfer_complete()` | XFER_COMPLETE (bit 1) | `cmd53_write_fixed()` 末尾 | 1 |

### 等待函数的内部结构：两阶段设计

所有四个等待函数最终都调用 `poll_int_status(bit)`（`lib.rs:157`）：

```
poll_int_status(bit):
  ┌─ Phase 1: 快速自旋 ──────────────────────────────────────┐
  │ for _ in 0..1000:                          // ~50µs on C906
  │   if bit 已在 INT_STATUS 中置位 → W1C 清除，立即返回 OK
  │   spin_loop()
  └──────────────────────────────────────────────────────────┘
  ┌─ Phase 2: 休眠/中断等待 ──────────────────────────────────┐
  │ for i in 0..20:                            // 最多 20 次
  │   if bit 已在 INT_STATUS 中置位 → 返回 OK
  │   ┌─ if bit == XFER_COMPLETE:
  │   │     动态使能硬件中断信号 (SIG_EN)
  │   │     block_timeout(10ms)  ← 被 ISR 唤醒或 10ms 超时
  │   │
  │   └─ else (CMD_COMPLETE / BUF_WR_READY / BUF_RD_READY):
  │         delay_ms(10)          ← 纯睡眠，无中断唤醒 ★
  └──────────────────────────────────────────────────────────┘
```

**Phase 1** 处理快速路径：硬件通常在微秒级完成——写几个寄存器后状态位就置位了。1000 次 `spin_loop()` 在 C906 @1GHz 上约 50µs。实测绝大多数等待（>90%）在 Phase 1 命中。

**Phase 2** 处理慢速路径：硬件偶尔因为 SDIO 总线忙、固件正在处理前一批数据等原因延迟就绪。此时有两种策略：
- **XFER_COMPLETE**（bit 1）：走硬件中断——`unmask_xfer_complete_signal()` 使能中断信号 → `block_timeout(10ms)` 阻塞 → ISR 检测到 XFER_COMPLETE 时调用 `pio_wake_callback` 唤醒任务。**微秒级响应**。
- **CMD_COMPLETE**（bit 0）/ **BUF_WR_READY**（bit 4）/ **BUF_RD_READY**（bit 5）：走纯超时睡眠——`delay_ms(10)` 后被动醒来检查。**固定 10ms 粒度**。

## `delay_ms(10)` 为什么是瓶颈

### 定量分析：一次 128KB TX 的等待成本

128KB TX = 256 blocks。一帧约 1500B（3 blocks），一次 128KB burst 约 85 帧。

每帧需等待：1 次 CMD_COMPLETE + 3 次 BUF_WR_READY + 1 次 XFER_COMPLETE = 5 次 poll_int_status。

85 帧共计：
- `wait_cmd_complete`: 85 次
- `wait_buffer_write_ready`: 255 次
- `wait_transfer_complete`: 85 次

**总计 425 次 `poll_int_status()` 调用。**

> 注：此计算适用于当前无聚合的逐帧发送。若实现 TX 聚合（参见 `sg2002-wifi-tx-official-driver-comparison.md` P1），85 帧合并为一次 CMD53，则等待次数降为 1 + 256 + 1 = 258 次——主要是 BUF_WR_READY。

即使 90% 在 Phase 1 命中（Phase 1 开销可忽略），10%（约 42 次）落入 Phase 2：

| 场景 | Phase 2 策略 | 额外等待时间 |
|------|-------------|------------|
| XFER_COMPLETE（bit 1） | 中断驱动 | ~42 × 0 = **0 ms** （中断唤醒，几乎立即） |
| CMD_COMPLETE（bit 0） | `delay_ms(10)` 睡眠 | ~9 次 × 10ms = **90 ms** |
| BUF_WR_READY（bit 4） | `delay_ms(10)` 睡眠 | ~26 次 × 10ms = **260 ms** |

**累积延迟：~350ms 用于等待硬件就绪**，而硬件本身只需要微秒级。这就是为什么 TX 吞吐从 Part 1 忙等方案的 ~10 Mbps 退化到当前的 ~0.85 Mbps——在 10 秒 iperf3 窗口中，约 35% 的时间在睡眠等待硬件，而非实际传输数据。

### 为什么 Phase 1 不能覆盖所有情况

Phase 1 自旋 1000 次（~50µs）在大多数情况下足够。但在以下场景中硬件会慢于此窗口：

- **CMD_COMPLETE 慢**：前一条命令（CMD53）还未完成（总线仍处于 CMD INHIBIT），控制器需要先完成前一条命令才能接受下一条。WiFi firmware 侧的 SDIO 处理延迟会传递到这里。
- **BUF_WR_READY 慢**：SDHCI 内部 FIFO（通常 512B×N）正在排水（将上一 block 的数据串行化到 SDIO 总线），填充下一 block 需等待排水完成。在 25MHz SDIO 时钟下，一个 512B block 的串行化约需 512×8/25MHz ≈ 164µs，快于 Phase 1 的 50µs 窗口，但多 block 连续写入时第一个 block 之后的间隔可能更短。

### 为什么不能用 1ms 睡眠代替 10ms

即使用 `delay_ms(1)` 代替 `delay_ms(10)`，上面的 350ms 变为 35ms，改善 10 倍——但 1ms 对于微秒级就绪的硬件来说仍然是 1000 倍的浪费。对于单核 TX 热路径，**CPU 没有其他有意义的工作可做**——它的唯一目标就是把数据推入 SDIO。自旋不浪费任何可被其他任务利用的 CPU 时间。

## 中断需求分析：除 XFER_COMPLETE 外还需要其他中断吗

### 当前的中断路由

`NORM_INT_SIG_EN` 寄存器的当前配置（`regs.rs:78`）：

```rust
pub const NORM_INT_SIG_MASK: u16 = NORM_INT_CARD_INT;
```

即：硬件中断线**只对 CARD_INT（bit 8）常开**。

`NORM_INT_STS_EN` 寄存器（`regs.rs:58-61`）：

```rust
pub const NORM_INT_ENABLE_MASK: u16 =
    NORM_INT_CMD_COMPLETE     // bit 0
    | NORM_INT_XFER_COMPLETE  // bit 1
    | NORM_INT_BUF_WR_READY   // bit 4
    | NORM_INT_BUF_RD_READY   // bit 5
    | NORM_INT_CARD_INT;      // bit 8
```

所有五个状态位都可以在 `INT_STATUS` 寄存器中锁存（STATUS ENABLE 全开），但只有 CARD_INT 的信号直通中断线（SIGNAL ENABLE 只开 bit 8）。

XFER_COMPLETE 的信号通过 `unmask_xfer_complete_signal()` 动态开关：task 阻塞前打开 → ISR 收到后关闭。这是一个**按需使能**的模式。

### ISR 的处理范围

`irq.rs:171-218` 的 `sdhci_irq_handler()` 只处理两种状态位：

1. **CARD_INT**（bit 8）：mask 信号 + 调用 `card_irq_callback` → 通知 WiFi 驱动有数据可读
2. **XFER_COMPLETE**（bit 1）：mask 信号 + 调用 `pio_wake_callback` → 唤醒阻塞的 PIO 任务

CMD_COMPLETE（bit 0）、BUF_WR_READY（bit 4）、BUF_RD_READY（bit 5）的硬件中断**不会触发 ISR**，因为它们的信号从未被使能。

### 是否应该为 CMD_COMPLETE / BUF_WR_READY 添加中断

**结论：在 SG2002 单核场景下，不建议。应采用自旋替代 `delay_ms(10)`。**

原因分析：

#### 自旋适合单核 SDHCI PIO 的原因

```
                    Phase 1 自旋 (~50µs)       Phase 2 自旋
Task:  [spin] [spin] [spin] [check] [ready!] → 继续写入 SDHCI_BUFFER
                     ↑ 微秒级响应
```

```
                    中断方案
Task:  [enable_irq] [block] ......... [ISR entry] [mask] [wake] → [resume]
                     ↑ ~10-100µs ISR 延迟          ↑ 再调度延迟
```

在单核 C906 上：
- ISR 有 entry/exit 开销（保存/恢复寄存器、PLIC claim/complete）
- `block_timeout()` → `WaitQueue::wait_timeout()` 有任务切换开销
- 被 ISR 唤醒后，任务需要等待调度器再次选中自己才能执行
- 对于预计在微秒级就绪的事件（BUF_WR_READY 通常在 160µs 内就绪），**中断的总开销超过了直接自旋的开销**

这就是为什么 SDHCI 标准驱动的 PIO 路径通常使用自旋轮询而非中断——Linux 内核的 `sdhci.c` 中 PIO 路径也是 `read_poll_timeout()` 循环读取 `PRESENT_STATE` 寄存器，不使用中断。

#### XFER_COMPLETE 走中断是合理的，因为

- XFER_COMPLETE 意味着整个传输（所有 block）完成，可能涉及较长延迟（多 block 串行化需数毫秒）
- 等待时间足够长，任务切换开销可被摊销
- 允许 CPU 在此期间处理其他事务（如 TCP 协议栈处理）

### CMD_COMPLETE 和 BUF_WR_READY 的自旋安全性

`wait_data_idle()`（`lib.rs:253`）已经使用纯自旋：

```rust
fn wait_data_idle(&self) -> Result<(), SdioError> {
    for _ in 0..CMD_RESPONSE_TIMEOUT {  // 100,000 次
        if self.read::<u32>(SDHCI_PRESENT_STATE) & (SDHCI_CMD_INHIBIT | SDHCI_DATA_INHIBIT) == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(SdioError::Timeout)
}
```

这里 `CMD_RESPONSE_TIMEOUT = 100_000` 次自旋**没有时间限制**，上限是 spin 次数而非时间。BUF_WR_READY 和 CMD_COMPLETE 的 Phase 1 只有 1000 次——关键在于 Phase 2 的策略选择。

将 Phase 2 从 `delay_ms(10)` 改为自旋意味着：
- 最坏情况：Phase 2 的 20 次迭代全部自旋 → 加到 Phase 1 的 1000 次 → 总自旋 = `1000 + 20 × 1000` = 21000 次（但这不是每 iteration 自旋 1000 次，而是每次检查后继续自旋）
- 但如果 Phase 2 也用 `spin_loop()` + recheck 模式，实际上和 `wait_data_idle` 的模式一致——N 次重试，每次检查寄存器后 `spin_loop()` 一下让 CPU hint
- 时间上限：如果硬件真的卡死了（DAT line stuck），纯自旋会永久卡住。需要保留超时作为安全网

**建议方案**：Phase 2 改为有限次数的自旋（如 100,000 次，与 `wait_data_idle` 一致），超过后仍走 `delay_ms(10)` 或报超时错误。这样：
- 正常路径：微秒级响应，无睡眠开销
- 异常路径：仍通过超时保护防止永久自旋

## 完整 PIO 等待路径修正方案

当前代码（`lib.rs:190-197`）：

```rust
if use_irq {
    irq::unmask_xfer_complete_signal();
    let _timed_out = crate::runtime::delay().block_timeout(PHASE2_STEP_MS);
} else {
    crate::runtime::delay().delay_ms(PHASE2_STEP_MS);
}
```

建议改为：

```rust
if use_irq {
    irq::unmask_xfer_complete_signal();
    let _timed_out = crate::runtime::delay().block_timeout(PHASE2_STEP_MS);
} else {
    // 单核 PIO 场景：自旋严格优于睡眠。BUF_WR_READY/CMD_COMPLETE 通常
    // 在微秒级就绪；即使硬件慢于 Phase 1，继续自旋的总开销也远低于
    // delay_ms(10) 的粗粒度睡眠。自旋次数对齐 wait_data_idle 的设计。
    for _ in 0..PHASE2_SPIN_ITERS {
        if let Some(result) = self.poll_status_once(bit) {
            return result;
        }
        core::hint::spin_loop();
    }
    // 达到自旋上限仍未就绪 → 回到睡眠模式作为最后安全网
    crate::runtime::delay().delay_ms(PHASE2_STEP_MS);
}
```

其中 `PHASE2_SPIN_ITERS` 可设为与 `wait_data_idle` 一致（100,000 次 ~5ms）或使用 `yield_now()` 作为中间步骤。

## 总结

| 问题 | 答案 |
|------|------|
| `delay_ms(10)` 在哪里 | `sdhci-cv1800/src/lib.rs:196`，非 XFER 位的 Phase 2 等待路径 |
| 影响哪些等待 | `wait_cmd_complete()`（CMD_COMPLETE bit 0）和 `wait_buffer_write_ready()`（BUF_WR_READY bit 4）、`wait_buffer_read_ready()`（BUF_RD_READY bit 5） |
| 一次 TX 触发多少次 | 每帧 ~85 次/128KB burst。以 10% Phase 2 命中率计，累积延迟 ~350ms |
| 为什么不用中断解决 | 单核上 ISR entry/exit + 任务切换的开销超过了微秒级就绪事件的自旋开销。自旋是合理选择 |
| XFER_COMPLETE 为什么已有中断 | XFER_COMPLETE 等待时间足够长（毫秒级），可摊销中断和调度开销 |
| 应该怎么修 | Phase 2 的非 XFER 路径：`delay_ms(10)` → 足够多次的自旋（如 100,000 次 `spin_loop()` + recheck），仅在自旋耗尽后落入睡眠作为安全网 |
| `wait_data_idle()` 作为参照 | 它已经使用纯自旋（100,000 次），BUF_WR_READY 和 CMD_COMPLETE 的自旋方案与之对齐 |
