# 代码审查问题的教学解释：Blockers 与 Should Fix

本文档对 `.ocr/sessions/2026-08-06-sg2002-wifi-irq/rounds/round-1/final.md`
中提出的 1 个 Blocker 和 6 个 Should Fix 做从零基础出发的教学解释。

**前置阅读**：建议先阅读 `wifi-sdio-irq-teaching.md` 了解整体硬件架构和中断机制，
再回到本文档理解每个具体问题。

---

## 第一部分：必须理解的基础概念

在深入具体问题之前，需要先建立几个关键概念。如果你已熟悉可以跳过。

### 1.1 什么是"寄存器"和"位"

SDHCI 控制器内部有一组**寄存器**（register），每个寄存器有固定的地址（地址 = 控制器
MMIO 基地址 + 偏移量）。比如偏移量 `0x30` 处的寄存器叫 `INT_STATUS_NORM`（普通中断
状态寄存器），它用 16 个 bit 表达 16 种中断的状态。

一个典型的 16-bit 寄存器：

```
bit:   15    14  ...  8    ...  4    3    2    1    0
     ┌────┬────┬─...─┬────┬─...─┬────┬────┬────┬────┐
     │ERR │    │     │CARD│     │BUF │    │    │XFER│CMD │
     │    │    │     │_INT│     │_WR │    │    │COMP│COMP│
     └────┴────┴─...─┴────┴─...─┴────┴────┴────┴────┘
```

每个 bit 像一个独立的旗子——置 1 表示"有事情发生了"，置 0 表示"没有"。

### 1.2 W1C（Write-1-to-Clear）：写 1 清零

这是 SDHCI 规范中最重要也是最反直觉的寄存器行为。

**正常直觉**：写 1 到某个 bit → 该 bit 变成 1。
**W1C 实际行为**：写 1 到某个 bit → 该 bit 变成 0（被清除）。

为什么要这样设计？考虑一个场景：
- 寄存器当前值是 `0b0000_0011`（bit0 和 bit1 都是 1）
- 中断处理程序只关心 bit0，想清除它
- 如果"写 0 清零"，需要写 `0b0000_0001`（只写 bit0=1，其余写 0）——但这也可能误清别的位
- 如果"写 1 清零"，写 `0b0000_0001`——bit0 被清为 0，bit1 保持不变

**W1C 的核心优势**：可以只清除你关心的位，不影响你不关心的位。你写进去的
哪些位是 1，就清除哪些位；写 0 的位保持原样。

```
当前寄存器值: 0b0000_0011  (bit0=1, bit1=1)
写 1 清零:    0b0000_0001  (只清 bit0)
结果:         0b0000_0010  (bit0 被清为 0，bit1 保持 1)
```

### 1.3 Sticky Bit（粘滞位）

**Sticky bit** 是一种特殊的中断状态位。普通的状态位会在触发条件消失后自动变回 0；
sticky bit 一旦被硬件置 1，就**一直保持 1**，直到软件显式地 W1C 清除它。

打个比方：
- **普通位**像门铃——按下时响，松开就停
- **Sticky bit** 像留言条——有人来过就会留下字条，你必须自己撕掉（W1C 清除）

`XFER_COMPLETE`（传输完成中断，bit1）就是一个 sticky bit。它被硬件置 1 后
不会自己消失，必须由软件 W1C 清除。

Sticky bit 在中断驱动设计中的价值：即使你在硬件置位和软件检查之间发生了各种
奇怪的事情（中断被屏蔽、任务被抢占等），sticky bit 仍然可靠地记录着"传输已完成"
这一事实。你可以稍后检查它而不会错过事件。

### 1.4 中断处理的四个寄存器

SDHCI 为普通中断状态提供了四个 16-bit 寄存器，它们分工明确：

| 寄存器 | 偏移 | 作用 | 类比 |
|--------|------|------|------|
| `INT_STATUS_NORM` | 0x30 | **状态**：哪些中断发生了（sticky） | 公告栏上的便签 |
| `NORM_INT_STS_EN` | 0x34 | **报告使能**：哪些中断允许出现在 STATUS 中 | 决定哪些便签可以贴到公告栏上 |
| `NORM_INT_SIG_EN` | 0x38 | **信号使能**：哪些中断会触发硬件 IRQ 线 | 决定哪些便签会触发警铃 |
| `NORM_INT_SIG_EN` (mask) | 0x38 | 写 0 到某位 = mask（屏蔽）该中断信号 | 关掉某个警铃触发器 |

关键区别：
- **Status Enable (0x34)**：控制是否记录事件。如果某位被 disable，即使硬件事件发生，
  `INT_STATUS` 中对应的 bit 也不会置 1。
- **Signal Enable (0x38)**：控制是否向外发送 IRQ 信号。即使 `INT_STATUS` 中
  某位置 1，如果对应 SIG_EN 位是 0，也不会向 CPU 发送中断请求。

这意味着：即使我们 mask 了信号（关警铃），status 位仍然可以被轮询观察到。

### 1.5 中断信号路径（端到端）

当 WiFi 芯片完成一次数据传输时，信号传播路径如下：

```
SDHCI 控制器检测到传输完成
  → INT_STATUS_NORM.XFER_COMPLETE 置 1 (sticky)
  → 如果 SIG_EN.XFER_COMPLETE = 1，拉高 IRQ 线
    → PLIC 中断控制器转发给 CPU
      → CPU 进入 ISR (sdhci_irq_handler)
        → ISR 读取 INT_STATUS_NORM
        → 发现 XFER_COMPLETE=1
        → mask 掉 SIG_EN 中的 XFER_COMPLETE（关警铃）
        → 调用 wake callback 唤醒阻塞的任务
          → 任务被唤醒，检查 INT_STATUS_NORM
          → 发现 XFER_COMPLETE=1
          → W1C 清除之（撕掉便签）
          → 返回成功
```

### 1.6 当前驱动中的 Phase 1 / Phase 2 架构

`poll_int_status` 函数（等待某个中断状态位置位）采用两阶段策略：

```
进入 poll_int_status(bit)
  │
  ├─ Phase 1: 快速自旋 (1000 次, ~50µs)
  │   每次循环读 INT_STATUS_NORM
  │   如果 bit 已置位 → 清除并返回成功
  │   如果错误位已置位 → 处理错误并返回失败
  │   否则 → spin_loop() 继续
  │
  └─ Phase 2: 慢速等待 (最多 20 次, 每次 10ms, 总计 200ms)
      每次循环：
        │
        ├─ Pre-check: 读 INT_STATUS_NORM（竞赛防护）
        │
        ├─ 如果是 XFER_COMPLETE:
        │   unmask XFER_COMPLETE 信号（开警铃）
        │   block_timeout(10ms)（睡眠，等 ISR 唤醒或超时）
        │
        ├─ 如果不是 XFER_COMPLETE:
        │   delay_ms(10ms)（纯睡眠，不经过中断唤醒）
        │
        └─ Post-check: 读 INT_STATUS_NORM
           如果 bit 已置位 → 清除并返回成功
           否则 → 继续下一轮循环

  20 次后仍未成功 → 返回 Timeout 错误
```

**为什么有两阶段？**
- Phase 1 覆盖绝大多数情况（响应时间通常在 µs 级别），用自旋避免上下文切换开销
- Phase 2 覆盖异常慢的情况（如硬件偶尔响应慢），用睡眠避免烧 CPU

**为什么 XFER_COMPLETE 走中断唤醒而其他位不走？**
- XFER_COMPLETE 等待时间长（要等整个数据块传输完，几十到几百 µs），容易错过 Phase 1
- CMD_COMPLETE/BUF_WR_READY/BUF_RD_READY 通常很快（几个 µs），Phase 1 足够
- ISR 只为 XFER_COMPLETE 注册了 wake callback，为其他位发 notify 也无意义

### 1.7 中断处理中的"mask 但不 clear"协议

这是当前驱动最重要的设计决策。看一下 ISR 中 XFER_COMPLETE 的处理：

```rust
// irq.rs:210-217
if norm & NORM_INT_XFER_COMPLETE != 0 {
    // 只 mask 信号，不清除 sticky status bit
    rmw_norm_sig_en(base, 0, NORM_INT_XFER_COMPLETE);
    // 唤醒阻塞的任务
    unsafe { SDHCI_IRQ_STATE.pio_wake_callback.invoke() };
}
```

ISR 做了两件事：
1. **Mask 信号**（关警铃，防止同一个事件反复触发中断）
2. **唤醒任务**（让任务去检查和清除 sticky bit）

ISR 刻意**不**做：
- **不清除 XFER_COMPLETE 的 sticky status bit**

为什么？如果 ISR 清除了 sticky bit，然后去唤醒任务，任务醒来后检查
`INT_STATUS_NORM` 就会发现 `XFER_COMPLETE=0`（已经被 ISR 清了），于是认为
传输还没完成，继续睡眠——进入 200ms 超时。

这就是"sticky bit 由任务端消费"协议：**ISR 负责通知，任务负责清除**。

---

## 第二部分：Blocker（必须修复）

### 🚫 Blocker 1: `clear_stale_status` 可能破坏 XFER_COMPLETE sticky bit

**严重程度**：必须修复。在双向流量下会导致丢帧和 200ms 停顿。

#### 问题场景

假设系统正在进行双向网络通信（例如你同时在上传和下载文件）：
- **TX 线程**正在发送数据，阻塞在 Phase 2 等待 `XFER_COMPLETE`
- **RX 线程**正在接收数据，频繁执行 CMD52/CMD53 命令

每次 RX 线程执行命令时，它首先调用 `clear_stale_status()`：

```rust
// lib.rs:310-322
fn clear_stale_status(&self) {
    let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
    if norm != 0 {
        // 清除错误状态...
        self.write::<u16>(SDHCI_INT_STATUS_NORM, norm);  // ← 关键：W1C 清除所有置位的 bit
    }
}
```

这个函数读 `INT_STATUS_NORM`，然后**把读到的值原样写回去**。根据 W1C 语义，
这意味着清除**所有**当前置位的状态位——包括 `XFER_COMPLETE`。

#### 时序窗口

关键的时间窗口如下：

```
时间 →

TX 线程                          RX 线程                     硬件
  │                                │                          │
  ├─ 发送 CMD53 写命令 ──────────────────────────────────────►│
  ├─ poll_int_status(XFER_COMPLETE)                           │
  │   Phase 1: 未完成                                         │
  │   Phase 2: unmask signal                                  │
  │   block_timeout(10ms) → 睡眠                              │
  │                                │                          │
  │                                ├─ CMD52 读 ──────────────►│
  │                                ├─ clear_stale_status()    │
  │                                │   INT_STATUS=0x0000      │
  │                                │   (XFER_COMPLETE 尚未置位)│
  │                                │                          ├─ TX 传输完成！
  │                                │                          │   XFER_COMPLETE=1
  │                                │                          │
  │                                ├─ CMD52 写 ──────────────►│
  │                                ├─ clear_stale_status()    │
  │                                │   INT_STATUS=0x0002 ────→│ ← 读到 XFER_COMPLETE=1！
  │                                │   W1C 写入 0x0002 ──────►│ ← 清除了 XFER_COMPLETE！
  │                                │                          │   XFER_COMPLETE=0
  │                                │                          │
  │  (10ms 后醒来)                 │                          │
  ├─ 检查 INT_STATUS_NORM          │                          │
  │   XFER_COMPLETE=0 ❌           │                          │
  ├─ "还没完成，继续等"             │                          │
  │   ...                          │                          │
  │   (又过 10ms，循环最多 20 次)   │                          │
  │   ...                          │                          │
  └─ 最终返回 Timeout               │                          │
     + DAT 线复位（200ms 总停顿）    │                          │
```

**问题本质**：TX 线程睡眠期间，RX 线程的 `clear_stale_status` 在硬件置位
`XFER_COMPLETE` 后、TX 线程醒来检查前，观察到了这个 sticky bit 并 W1C 清除了它。
TX 线程醒来后看到的 `INT_STATUS_NORM` 中 `XFER_COMPLETE` 已经是 0——传输完成的
事实被抹除了。

#### 为什么旧代码没有这个问题？

旧代码的 Phase 2 使用 `yield_now()`（忙等 + 让出 CPU），每次循环只有几 µs。
在这么短的窗口中，另一个线程恰好插入 `clear_stale_status` 的概率极低，
可以忽略。

新代码的 Phase 2 使用 `block_timeout(10ms)`——每次睡眠 **10 毫秒**。
在这个时间尺度上，RX 线程几乎肯定会在 TX 睡眠期间执行多条命令，每条命令
都调用 `clear_stale_status`。触发概率从"几乎不可能"变成"几乎必然"。

#### 修复方案

让 `clear_stale_status` 不触碰 `XFER_COMPLETE`（也不触碰 `CARD_INT`）：

```rust
fn clear_stale_status(&self) {
    let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
    if norm != 0 {
        if norm & NORM_INT_ERROR != 0 {
            let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
            if err != 0 {
                self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
            }
        }
        // 只清除非 XFER_COMPLETE 和非 CARD_INT 的位
        // XFER_COMPLETE: 由 poll_int_status 独占消费
        // CARD_INT:     由 ISR/mask 协议独占管理
        let safe_clear = norm & !(NORM_INT_XFER_COMPLETE | NORM_INT_CARD_INT);
        if safe_clear != 0 {
            self.write::<u16>(SDHCI_INT_STATUS_NORM, safe_clear);
        }
    }
}
```

同时审查 `prepare_first_data_xfer`（lib.rs:325-330）是否需要同样的修复——
它写 `0xFFFF` 清除所有位。当前仅在 probe 期间调用，风险较低，但为了一致性
也应该对齐选择性清除策略。

#### 为什么这是 Blocker

1. **违反设计协议**：`irq.rs` 模块文档明确声明"sticky bit 仅由 poll_int_status 消费"，
   `clear_stale_status` 破坏了这个不变量
2. **导致真实的数据丢失**：每次触发导致丢一帧 + 200ms 停顿
3. **双向流量必然触发**：任何有并发 RX 活动的 TX 场景
4. **难以调试**：间歇性、概率性的时序 bug，日志难以捕获

---

## 第三部分：Should Fix（应该修复）

### Should Fix 1: TX/RX 并发阻塞时的 Wake-to-Waiter 匹配问题

**严重程度**：当前由 transport 锁范围保证安全，但该保证不可见且脆弱。

#### 背景：WaitQueue 的工作方式

`WaitQueue` 是一个等待队列。多个任务可以同时阻塞在同一个队列上，调用
`notify_one()` 会唤醒**队首的第一个任务**（FIFO 顺序），而不是某个特定任务。

```rust
// wifi_glue.rs:68
static SDHCI_PIO_WQ: WaitQueue = WaitQueue::new();

// wifi_glue.rs:70-72
fn sdhci_pio_wake_callback() {
    SDHCI_PIO_WQ.notify_one_from_irq();  // 唤醒队首，不是特定任务
}
```

#### 问题

TX 和 RX 线程都可能阻塞在同一个 `SDHCI_PIO_WQ` 上等待 `XFER_COMPLETE`。
当 ISR 触发 `notify_one` 时，它只唤醒队首任务——而这个任务可能**不是**真正
完成传输的那个。

```
情况：
  1. TX 任务先阻塞 → 在 WaitQueue 的队首
  2. RX 任务后阻塞 → 在 WaitQueue 的第二个位置
  3. RX 的传输完成 → ISR 调用 notify_one → 唤醒队首的 TX 任务
  4. TX 醒来检查 INT_STATUS_NORM → 发现 XFER_COMPLETE=1（但这是 RX 的！）
  5. TX 消费掉 XFER_COMPLETE → 返回"成功"
  6. RX 的 XFER_COMPLETE 被 TX 消费了 → RX 等待 10ms 后超时或等下一轮
```

#### 为什么当前不出问题

因为 transport 层的锁（`Mutex`）保证了同一时间只有一个线程能进入 SDIO 传输路径。
**TX 和 RX 不可能同时阻塞在 WaitQueue 上**——当一个在传输时，另一个在等锁。

但这个保证是**隐式的**——它不在 `SdhciDelay` trait 的文档中，不在 `WaitQueue`
的类型签名中，也不在任何 `debug_assert` 中。如果未来有人修改了锁的范围，或者
增加了新的传输路径，这个隐式保证就会静默失效。

#### 修复建议

1. 在 `SdhciDelay::block_timeout` 的 trait 文档中明确声明"同一时间最多一个 waiter"
2. 添加一个 occupancy flag 和 `debug_assert`，检测是否已有任务在阻塞

### Should Fix 2: 单核假设仅由 debug_assert 保护

**严重程度**：release 构建中静默失效，SMP 场景下表现为间歇性丢帧。

#### 背景：为什么单核重要

当前驱动的 SIG_EN RMW 协议基于一个关键假设：**只有一个 CPU 核在运行**。

```rust
// irq.rs:144-148
fn rmw_norm_sig_en(base: usize, set: u16, clear: u16) {
    let addr = base + SDHCI_NORM_INT_SIG_EN as usize;
    let cur = mmio_read::<u16>(addr);      // 读
    mmio_write::<u16>(addr, (cur & !clear) | set);  // 修改 + 写
}
```

这是"读-修改-写"（RMW）模式。在单核上，唯一的并发危险是中断抢占（ISR 在读和写
之间执行）。当前设计依赖 XFER_COMPLETE sticky bit 自愈——即使 SIG_EN 被错误覆写，
sticky bit 仍然置位，中断线会在任务重新阻塞后重新断言。

在 SMP（多核）上，两个核可能**同时**执行 RMW：
- Core 1 读 SIG_EN = 0x0100（CARD_INT 使能）
- Core 2 读 SIG_EN = 0x0100
- Core 1 写 SIG_EN = 0x0102（CARD_INT + XFER_COMPLETE）
- Core 2 写 SIG_EN = 0x0100（基于过期值，丢失了 Core 1 的 XFER_COMPLETE）

sticky bit 自愈仍然有效，但每次 RMW 冲突都可能导致额外 10ms 停顿。

#### 当前防护

```rust
// wifi_glue.rs:95-98
debug_assert!(
    ax_hal::cpu_num() == 1,
    "sdhci-cv1800 ISR design assumes single-core; SMP not yet supported"
);
```

`debug_assert!` 在 release 构建中被编译器移除！这意味着在生产代码中，
如果有人在 SMP 构建中使用此驱动，没有任何防护——表现为难以复现的间歇性停顿。

#### 修复建议

升级为硬 `assert!`（release 中也保留），或使用编译期 `#[cfg]` 门控：

```rust
// 方案 A: 硬 assert
assert!(
    ax_hal::cpu_num() == 1,
    "sdhci-cv1800 ISR design assumes single-core; SMP not yet supported"
);

// 方案 B: 编译期门控（更彻底）
#[cfg(any(feature = "smp", not(target_arch = "riscv64")))]
compile_error!("sdhci-cv1800 driver currently only supports single-hart RISC-V");
```

### Should Fix 3: 非 XFER 位的 Phase 2 10ms 睡眠惩罚

**严重程度**：在枚举和冷路径中造成显著的启动延迟，但不影响稳态吞吐。

#### 问题

Phase 2 对所有非 `XFER_COMPLETE` 的等待使用 10ms 固定步长：

```rust
// lib.rs:207-211
if use_irq {
    irq::unmask_xfer_complete_signal();
    let _timed_out = crate::runtime::delay().block_timeout(PHASE2_STEP_MS);
} else {
    // 非 XFER 位：纯睡眠 10ms
    crate::runtime::delay().delay_ms(PHASE2_STEP_MS);
}
```

Phase 1 的 1000 次自旋大约覆盖 ~50µs。以下场景的事件耗时超过 ~50µs：

| 场景 | 典型耗时 | Phase 1 能捕获？ |
|------|---------|------------------|
| 25MHz CMD_COMPLETE | ~5µs | ✓ |
| 400kHz 枚举 CMD5/3/7 | ~240µs | ✗ → 每次 10ms |
| 400kHz probe CMD52 | ~150µs | ✗ → 每次 10ms |
| 25MHz 512B FIFO drain | ~82µs | ✗ → 10ms |
| probe 窗口（PLIC IRQ 未使能） | — | XFER 也走 10ms |

**具体影响**：
- **启动时间**：枚举阶段 CMD5 轮询 + CMD3 + CMD7 + 约 20 个 CMD52 = 约 20-30
  个命令错过 Phase 1 → +200-300ms 启动时间
- **probe 阶段**：IRQ 未使能期间，所有 XFER 等待也走 10ms——每个 CMD53 付出 10ms

旧代码的 `yield_now` 循环在 µs 级别就能捕获这些事件。新代码引入了 10ms 的粒度悬崖。

#### 修复建议

1. **降低步长**：非 XFER 路径使用 1ms 步长（1ms × 200 迭代），总预算不变
2. **添加观测性**：在 Phase-2 入口添加 `log::trace!`，记录哪些位、哪些命令进了 Phase 2
3. **文档化权衡**：10ms 的 100% 延迟换取 0% CPU 占用——这是一个有意的取舍，
   但应该在代码中显式说明

### Should Fix 4: ISR 链中冗余的 CARD_INT mask

**严重程度**：功能正确但浪费，注释与实现矛盾。

#### 问题

CARD_INT 的 mask 被执行了两次：

```rust
// 第一次：sdhci_irq_handler (irq.rs:193-198)
if norm & NORM_INT_CARD_INT != 0 {
    mask_card_irq_raw(base, true);  // ← mask CARD_INT
    unsafe { SDHCI_IRQ_STATE.card_irq_callback.invoke() };  // → 调用 aic8800 的回调
}

// 第二次：aic8800 的回调 (bus.rs:316-326)
// sdio1_irq_handler → mask_card_irq() → CviCardIrqCtrl::mask_card_irq()
//   → irq::mask_card_irq_raw(base, true)  // ← 又 mask 一次
```

这是对同一个 SIG_EN 寄存器的第二次冗余 RMW——在同一个 ISR 调用链内。

同时，`rx.rs:274` 的注释声称"ISR 只设 flag 不 mask"，与两方面的实现矛盾。

#### 修复建议

从 aic8800 侧移除 mask（因为 SDHCI ISR 已经做了），保持 SDHCI ISR 为 mask 的
唯一权威方。同时修正 `rx.rs` 中的注释。

### Should Fix 5: poll_int_status 文档注释过时

**严重程度**：文档误导，不影响功能。

#### 问题

`poll_int_status` 的文档仍写：

```rust
/// 直接轮询 INT_STATUS_NORM，等待指定 bit 置位后 W1C 清除所有状态位
```

但实际代码（通过 `poll_status_once` 和超时路径）已经改为**选择性清除**——
只清除错误位和当前等待位，保留 `CARD_INT`。

#### 修复建议

更新文档反映实际行为：

```rust
/// 轮询 INT_STATUS_NORM，等待指定 bit 置位后选择性 W1C 清除。
///
/// 清除策略：
/// - 正常路径：仅清除等待的目标位
/// - 错误路径：清除 ERROR + 目标位，保留 CARD_INT
/// - 超时路径：清除 ERROR + 目标位 + DAT 线复位，保留 CARD_INT
```

### Should Fix 6: CARD_INT 不变量被兄弟代码矛盾

**严重程度**：与 Blocker 1 同类，应一起修复。

#### 问题

`clear_int_status_norm` 的文档声明：

```rust
/// Never clears CARD_INT — that is exclusively managed by the ISR/mask protocol.
fn clear_int_status_norm(&self, bits: u16) {
```

但 `clear_stale_status` 和 `prepare_first_data_xfer` 仍然 W1C 清除所有位，
包括 `CARD_INT`。这与 Blocker 1 的 `XFER_COMPLETE` 问题是同一类型：一个函数
承诺"永远不碰某个位"，但其他函数不遵守这个承诺。

#### 修复建议

与 Blocker 1 一起修复：使所有命令路径的清除统一为选择性策略。
在模块文档中建立清晰的位所有权模型：

| 位 | 所有者（谁有权清除） |
|----|---------------------|
| CMD_COMPLETE | poll_int_status (task) |
| XFER_COMPLETE | poll_int_status (task) |
| BUF_WR_READY | poll_int_status (task) |
| BUF_RD_READY | poll_int_status (task) |
| CARD_INT | ISR/mask 协议 |
| ERROR | poll_int_status (task) + clear_stale_status |

---

## 第四部分：总结

### 问题严重程度排序

| 优先级 | 问题 | 影响 |
|--------|------|------|
| 🔴 Blocker | `clear_stale_status` 清除 XFER_COMPLETE | 双向流量丢帧 + 200ms 停顿 |
| 🟡 Should Fix #1 | Wake-to-waiter 匹配 | 隐式假设，未来可能失效 |
| 🟡 Should Fix #2 | debug_assert 单核检查 | release 构建无保护 |
| 🟡 Should Fix #3 | 10ms Phase 2 步长 | 枚举/boot 延迟 |
| 🟡 Should Fix #6 | CARD_INT 不变量矛盾 | 与 Blocker 1 同类 |
| 🟢 Should Fix #4 | 冗余 CARD_INT mask | 浪费，注释不一致 |
| 🟢 Should Fix #5 | 文档过时 | 误导 |

### 根因分析

多数问题的根因可以追溯到同一个模式：**选择性清除策略没有被一致地应用**。

代码中有一个正确的核心函数 `clear_int_status_norm`（文档声明了"不碰 CARD_INT"），
也有正确的核心函数 `poll_status_once`（实现了选择性清除），但外围函数
`clear_stale_status` 和 `prepare_first_data_xfer` 仍然使用旧的"全部清除"策略。

这本质上是一个**增量重构中的一致性遗漏**：核心路径被仔细重新设计了，
但辅助路径没有同步更新。

### 修复顺序建议

1. **先修 Blocker 1**：让 `clear_stale_status` 采用选择性清除（同时修 Should Fix #6）
2. **再修 Should Fix #2**：升级单核断言为硬 assert
3. **然后修 Should Fix #1 和 #3**：添加文档和观测性
4. **最后修 Should Fix #4 和 #5**：清理冗余和文档
