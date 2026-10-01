# SDHCI XFER_COMPLETE 中断驱动 PIO 传输完成流程

本文档描述 `sg2002/wifi-irq` 分支在 `sdhci-cv1800` 组件中引入的中断驱动 PIO 传输完成机制，涵盖从 ISR 注册到任务唤醒的完整数据流，以及各实现维度的设计决策。

## 背景

SG2002 (CV1800B) 的 SDHCI 控制器通过 PIO 模式与 AIC8800 WiFi 芯片通信。每次 CMD53 数据传输完成后，硬件在 `INT_STATUS_NORM` 寄存器中置位 `XFER_COMPLETE` (bit 1)。原实现使用纯轮询等待该位，典型延迟为数毫秒到数十毫秒。中断驱动方案通过 PLIC 硬件中断唤醒阻塞任务，将传输完成到任务恢复的延迟降至中断响应时间（微秒级）。

## 整体流程

```
CMD53 数据传输完成
  │
  ▼
硬件: INT_STATUS_NORM.XFER_COMPLETE = 1
  │
  ▼
PLIC → sdhci_irq_handler()          [irq.rs]
  │
  ├─ 读 INT_STATUS_NORM
  ├─ mask XFER_COMPLETE SIG_EN       (防止 ISR 重复触发)
  └─ pio_wake_callback.invoke()
       │
       ▼
     sdhci_pio_wake_callback()      [wifi_glue.rs]
       │
       └─ SDHCI_PIO_WQ.notify_one_from_irq()
            │
            ▼
         唤醒 ArceosDelay::block_timeout()  [wifi_glue.rs]
            │
            ▼
         poll_int_status() recheck  [lib.rs]
            │
            └─ W1C 清除 INT_STATUS_NORM.XFER_COMPLETE (由任务端消费)
```

## 实现维度

### 1. CallbackSlot — ISR 回调注册机制

**文件:** `components/sdhci-cv1800/src/irq.rs:26-60`

`CallbackSlot` 是一个零分配、ISR 安全的函数指针槽：

- **存储:** `AtomicUsize`，零值表示未注册。`fn()` 在所有支持平台（riscv64、aarch64、x86_64）上均为指针宽度，可直接通过 `transmute` 在 `usize` 和 `fn()` 之间转换。
- **注册:** `register()` 在初始化阶段、IRQ 使能前调用一次，以 `Release` 语义写入。
- **调用:** `invoke()` 在 ISR 上下文中以 `Acquire` 语义加载，零值防护避免空槽调用。回调在硬中断上下文执行，禁止持锁、分配堆、调度、调用同步写 UART 的 `log` 宏。
- **单核假设:** 注册只写一次（初始化时），ISR 只读，无并发写入。

两个 `CallbackSlot` 实例分别服务于：
| 槽 | 注册函数 | 用途 |
|---|---|---|
| `card_irq_callback` | `register_card_irq_callback()` | 通知 WiFi 驱动有数据可读 |
| `pio_wake_callback` | `register_pio_wake_callback()` | 唤醒阻塞在 `block_timeout` 的任务 |

### 2. sdhci_irq_handler — ISR 入口

**文件:** `components/sdhci-cv1800/src/irq.rs:173-212`

注册到 PLIC 的 SDHCI 中断处理函数，处理两种中断：

**CARD_INT 路径 (line 189-195):**
1. mask 信号（`mask_card_irq_raw(base, true)` — 对 SIG_EN 做 RMW 清零 CARD_INT 位）
2. 调用 `card_irq_callback` 通知上层

**XFER_COMPLETE 路径 (line 204-211):**
1. mask 信号（`rmw_norm_sig_en(base, 0, NORM_INT_XFER_COMPLETE)` — 对 SIG_EN 写 0）
2. 调用 `pio_wake_callback` 唤醒阻塞任务
3. **不在 ISR 中 W1C 清除 sticky 状态位** — 注释明确说明（line 197-203）：
   - 若 ISR 清除状态位，被唤醒任务的 recheck 将看不到该位，破坏唤醒条件并导致必然的 200ms 超时
   - 状态位由被唤醒任务在 `poll_status_once` 中观察并 W1C 消费

**关键设计决策 — ISR 只 mask SIG_EN 不碰 STATUS：**

这是整个中断协议的核心。ISR 的职责是"通知任务有事件发生"，而非"消费事件"。sticky 状态位由任务端在 recheck 时通过 W1C 清除，确保唤醒-消费的原子性。

### 3. SDHCI_PIO_WQ — 共享唤醒队列

**文件:** `os/arceos/modules/axruntime/src/wifi_glue.rs:68-74`

```rust
static SDHCI_PIO_WQ: WaitQueue = WaitQueue::new();
```

- 全局静态 `WaitQueue`，至多一个任务同时阻塞在此队列上。
- **单 waiter 不变量:** SDIO 总线锁（`SdioTransport`）序列化所有传输，任意时刻仅 TX 或 RX 线程之一可处于 `block_timeout` 中。
- 使用 `notify_one_from_irq()` 而非 `notify_all()` — 始终只有一个 waiter。

### 4. sdhci_pio_wake_callback — 唤醒回调

**文件:** `os/arceos/modules/axruntime/src/wifi_glue.rs:76-78`

```rust
fn sdhci_pio_wake_callback() {
    SDHCI_PIO_WQ.notify_one_from_irq();
}
```

- 零分配、零 panic 路径的函数，仅在 ISR 上下文中被 `CallbackSlot::invoke()` 调用。
- 注册发生在 `install_runtime()` 中（line 107）：`sdhci_cv1800::irq::register_pio_wake_callback(sdhci_pio_wake_callback)`。

### 5. block_timeout — 中断驱动阻塞等待

**文件:** `os/arceos/modules/axruntime/src/wifi_glue.rs:88-90`

```rust
fn block_timeout(&self, timeout_ms: u64) -> bool {
    SDHCI_PIO_WQ.wait_timeout(Duration::from_millis(timeout_ms))
}
```

- 替换默认的纯 sleep 实现（`runtime.rs:23-26`），改为在共享 `WaitQueue` 上阻塞。
- 返回 `true` 表示超时，`false` 表示被 ISR 唤醒。
- 超时值 `PHASE2_STEP_MS = 10ms`，最多 `PHASE2_MAX_ITERS = 20` 次迭代（总预算 200ms）。

### 6. SdhciDelay trait — OS 能力注入

**文件:** `components/sdhci-cv1800/src/runtime.rs:11-27`

```rust
pub trait SdhciDelay: Send + Sync + 'static {
    fn delay_ms(&self, ms: u64);
    fn block_timeout(&self, timeout_ms: u64) -> bool;
}
```

- `delay_ms`: 纯阻塞延迟。
- `block_timeout`: 阻塞当前任务直至硬件中断唤醒或超时。默认实现回退到 `delay_ms`（纯 sleep），兼容未更新的 OS 胶水层。
- **单 waiter 契约 (line 17-21):** 调用方保证至多一个任务同时阻塞。此契约使 OS 胶水层可安全使用单一共享唤醒队列，丢失唤醒由超时兜底。
- 安装通过 `set_delay()` 在初始化时一次性完成，之后驱动通过 `delay()` 访问。

### 7. SIG_EN RMW 协议 — task 与 ISR 的协同

**文件:** `components/sdhci-cv1800/src/irq.rs:130-165`

SIG_EN 的所有 read-modify-write 操作集中在 `rmw_norm_sig_en()` 函数（line 140-144）：

| 调用方 | 操作 | 位置 |
|---|---|---|
| `unmask_xfer_complete_signal()` | set XFER_COMPLETE, clear 0 → **写 1（使能）** | line 155 |
| `sdhci_irq_handler` (XFER 路径) | set 0, clear XFER_COMPLETE → **写 0（屏蔽）** | line 206 |
| `mask_card_irq_raw(mask=true)` | set 0, clear CARD_INT → **写 0（屏蔽）** | line 161 |
| `mask_card_irq_raw(mask=false)` | set CARD_INT, clear 0 → **写 1（恢复）** | line 163 |

**RMW 竞态与自愈 (line 10-16 模块文档):**

SIG_EN 的 RMW 不是原子的——ISR 可能在 task 的 `mmio_read` 和 `mmio_write` 之间抢占。此竞态由 XFER_COMPLETE sticky bit 自愈：即使 SIG_EN 被错误重写，电平触发的中断线在 task 阻塞后重新断言，ISR 重新触发。最坏情况退化为一次 10ms 超时。SMP 平台需额外围栏。

### 8. poll_int_status — 两阶段等待

**文件:** `components/sdhci-cv1800/src/lib.rs:160-239`

**Phase 1 (line 172-177):** 快速自旋轮询 `INT_STATUS`（~50µs on C906 @1GHz）。在进入循环前执行 `SeqCst` fence（line 169）排空存储缓冲区，确保 PIO 写入在轮询开始前已被硬件接收。

**Phase 2 (line 185-219):** XFER_COMPLETE 走中断驱动等待，其他位（CMD_COMPLETE、BUF_RD_READY、BUF_WR_READY）走纯超时 sleep：
- 每轮迭代前执行 race guard 检查（pre-check，line 189）——先查状态再阻塞
- `use_irq = bit == NORM_INT_XFER_COMPLETE`（line 180）
- 中断路径：`unmask_xfer_complete_signal()` → `block_timeout(10ms)`（line 207-208）
- 非中断路径：`delay_ms(10ms)`（line 212）
- 被唤醒后 recheck 状态寄存器（line 217）——此步骤消费 sticky 状态位

### 9. 选择性 W1C — 保护 XFER_COMPLETE 不被错误清除

**文件:** `components/sdhci-cv1800/src/lib.rs`

三个关键位置实现选择性 W1C：

**`clear_stale_status()` (line 319-333):** 在发送命令前清除残留 INT_STATUS，但 mask 掉 XFER_COMPLETE——它可能被阻塞在 `poll_int_status` Phase 2 的任务消费。若此处清除，任务的 recheck 将永远看不到该位。

**`poll_status_once()` 错误分支 (line 141):** 检测到错误时，选择性清除 `NORM_INT_ERROR | bit | NORM_INT_XFER_COMPLETE`。XFER_COMPLETE 可能与错误同时置位（如数据阶段完成后发生 DAT 错误），在此消费可防止 stale bit 泄漏至下一传输。

**`poll_int_status()` 超时分支 (line 236):** 超时后清除 `NORM_INT_ERROR | bit | NORM_INT_XFER_COMPLETE` 并复位 DAT 线，防止总线被焊死。

### 10. Store Buffer Fence — Phase 1 前的内存栅栏

**文件:** `components/sdhci-cv1800/src/lib.rs:161-169`

在 Phase 1 自旋循环前插入 `core::sync::atomic::fence(SeqCst)`。若无此栅栏，待处理的 MMIO 写（如 `pio_write` 的 128 次 `SDHCI_BUFFER` 写入）可能仍排在 CPU 存储缓冲区中，而 `mmio_read(INT_STATUS_NORM)` 循环已开始。读取与排空写入在 SDHCI 总线上竞争，导致 Phase 1 的 1000 次迭代窗口在状态位可见前过期，落入 10ms Phase 2 延迟。

## 提交列表

| 提交 | 说明 |
|---|---|
| `7c74a3ca2` | 修复 WiFi 启动流程，引入 SG2002 WiFi 配置 |
| `a9a2fce27` | **核心提交:** 引入中断驱动 PIO 传输完成机制（CallbackSlot、ISR、WakeQueue、block_timeout） |
| `4ec9d3ec3` | 添加 per-bit poll_int_status 诊断计数器（后续被 revert） |
| `32d28faa1` | Revert 诊断计数器 |
| `d1af2e93d` | 在 Phase 1 MMIO 轮询前插入 store buffer fence |
| `cb0fb20e4` | 使用选择性 W1C 保护 XFER_COMPLETE 不被命令和错误路径破坏 |
| `75d32b71b` | 在错误和超时退出路径中消费 XFER_COMPLETE，防止 stale bit 泄漏 |
| `c61334d3a` | 将新增注释翻译为中文以保持一致性 |
| `14c0d6c68` | 从 licheerv-nano-sg2002 基础配置中移除 aic8800-wifi feature |
