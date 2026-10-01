# SG2002 WiFi SDIO 驱动与中断机制教学文档

本文档从零开始，逐步解释 SG2002 平台上 AIC8800 WiFi 芯片驱动的工作原理，
涵盖硬件基础、寄存器、驱动架构、中断机制、数据通路。

---

## 1. 硬件概览

### 1.1 三块芯片、两条总线

在我们的板子上，WiFi 功能涉及三个硬件：

```
┌──────────────┐     SDIO 总线      ┌──────────────┐
│  SG2002 SoC  │ ◄────────────────► │ AIC8800 WiFi │
│  (RISC-V)    │   CMD+DAT0-3+CLK   │   芯片       │
│              │                    │              │
│  ┌────────┐  │                    │  内部运行    │
│  │SDHCI   │  │                    │  FullMAC     │
│  │控制器  │  │                    │  固件        │
│  └────────┘  │                    │              │
│  ┌────────┐  │                    │  管理 802.11 │
│  │ PLIC   │  │                    │  协议栈      │
│  │中断控制器│ │                    │              │
│  └────────┘  │                    └──────────────┘
└──────────────┘
```

- **SG2002 SoC**（主控芯片）：一颗 RISC-V 64 位处理器，内部集成了 SDHCI 控制器和 PLIC 中断控制器
- **AIC8800**（WiFi 芯片）：一颗 FullMAC WiFi 芯片，内部运行自己的固件，自己管理 802.11 协议栈
- **SDIO 总线**：连接 SoC 和 WiFi 芯片的物理通道，6 根线（CMD=命令，CLK=时钟，DAT0~3=数据）

> **FullMAC 是什么意思？**
> WiFi 芯片分两种：SoftMAC 和 FullMAC。SoftMAC 芯片只做物理层，802.11 协议栈（关联、认证、加密等）跑在 CPU 上。FullMAC 芯片自己跑固件管理所有 802.11 事务，CPU 只需通过 SDIO 发"帮我连这个 AP"之类的命令。
> AIC8800 是 FullMAC，所以我们不用在 CPU 上跑 802.11 状态机。

### 1.2 SDHCI 控制器是什么

SDHCI（SD Host Controller Interface）是 SD 协会定义的一套标准。SoC 内部有一块硬件电路叫"SDHCI 控制器"，它负责：

1. **把 CPU 的 MMIO 操作翻译成 SDIO 总线信号**。CPU 写某个寄存器 → 控制器在 SDIO 总线上发出相应命令
2. **管理 SDIO 协议细节**：时钟分频、CRC 校验、超时检测
3. **产生中断**：当 SDIO 总线上发生事件时（命令完成、传输完成、卡有数据……），置位状态寄存器并可选地触发 CPU 中断

### 1.3 PLIC 中断控制器

PLIC（Platform-Level Interrupt Controller）是 RISC-V 平台的中断路由管理器。它的工作很简单：

- 外部硬件（SDHCI 控制器、定时器等）各有一条中断线连接到 PLIC
- PLIC 管理优先级，把中断路由到某个 CPU 核心
- 我们的 SDHCI 控制器在 PLIC 上的编号是 **IRQ #38**

当 SDHCI 控制器拉高中断信号 → PLIC 决定路由给 CPU → CPU 跳转到中断处理函数。

---

## 2. SDHCI 寄存器

SDHCI 控制器通过一组 MMIO 寄存器与 CPU 交互。CPU 读写这些寄存器来发命令、收数据、处理中断。

### 2.1 寄存器布局

寄存器位于 SoC 物理地址 `0x04320000`，在驱动中映射为 Rust 结构体的 `base` 指针 + 偏移量：

| 偏移 | 名称 | 宽度 | 作用 |
|------|------|------|------|
| 0x08 | ARGUMENT | 32-bit | 命令参数（如 SDIO 地址） |
| 0x0C | TRANSFER_MODE | 16-bit | 传输模式（多块、方向） |
| 0x0E | COMMAND | 16-bit | 命令索引 + 响应类型标志 |
| 0x10 | RESPONSE | 32-bit | 命令响应值 |
| 0x20 | BUFFER | 32-bit | 数据端口（PIO 读写 FIFO 的窗口） |
| 0x24 | PRESENT_STATE | 32-bit | 当前总线状态（CMD 忙？DAT 忙？缓冲区可写？） |
| 0x2C | CLOCK_CONTROL | 16-bit | 时钟使能、分频器、稳定标志 |
| 0x2F | SOFTWARE_RESET | 8-bit | 软件复位（全复位 / CMD 线复位 / DAT 线复位） |

以及中断相关的寄存器（见 §2.2）。

> **MMIO 是什么？**
> MMIO（Memory-Mapped I/O）就是把硬件寄存器映射到内存地址空间。CPU 用普通的 load/store 指令读写某个地址，实际访问的是硬件寄存器而不是内存。在 Rust 中用 `read_volatile` / `write_volatile` 实现，确保编译器不会优化掉这些"看起来没用"的读写。

### 2.2 中断寄存器（核心）

中断系统由三组寄存器控制，这是理解整个中断机制的关键：

```
偏移 0x30: INT_STATUS_NORM  (16-bit 读/写1清)  ← 状态：哪些事件发生了
偏移 0x32: INT_STATUS_ERR   (16-bit 读/写1清)  ← 错误状态
偏移 0x34: NORM_INT_STS_EN  (16-bit 读/写)     ← 状态使能：哪些位进 STATUS
偏移 0x36: ERR_INT_STS_EN   (16-bit 读/写)     ← 错误状态使能
偏移 0x38: NORM_INT_SIG_EN  (16-bit 读/写)     ← 信号使能：哪些位产生中断
偏移 0x3A: ERR_INT_SIG_EN   (16-bit 读/写)     ← 错误信号使能
```

**三级漏斗模型**：

```
硬件事件（如传输完成）
  │
  ▼
┌─────────────────────────────┐
│ ① STATUS 寄存器             │
│   硬件置位，软件 W1C 清除    │  ← 锁存型（sticky）：一旦硬件置 1，保持直到软件写 1 清除
│   无论下面两级怎么配，这一级 │
│   都会记录                       │
└────────────┬────────────────┘
             │ 经过 STS_EN 过滤
             ▼
┌─────────────────────────────┐
│ ② STATUS ENABLE (STS_EN)    │
│   控制哪些位能进入 STATUS    │  ← 通常全开，让所有事件都记录
│   设 0 = 屏蔽，事件不进 STATUS │
└────────────┬────────────────┘
             │ 经过 SIG_EN 过滤
             ▼
┌─────────────────────────────┐
│ ③ SIGNAL ENABLE (SIG_EN)   │
│   控制哪些位产生硬件中断      │  ← 选择性开，只让关心的事件触发 ISR
│   设 0 = 只记录在 STATUS，   │
│   不触发 CPU 中断            │
│   设 1 = 拉高中断线 → PLIC   │
└─────────────────────────────┘
```

**关键概念**：

- **STATUS 是锁存型（sticky）**：比如 XFER_COMPLETE 位，硬件完成传输后把它置 1，它就一直是 1，直到软件写 1 清除（Write-1-to-Clear, W1C）。在这期间无论读多少次都是 1。
- **SIG_EN 控制是否触发 ISR**：如果某位在 SIG_EN 中为 0，即使 STATUS 中对应的位是 1，也不会产生中断。但 STATUS 位仍然可以被轮询读到。
- **中断是电平触发的**：只要 `STATUS & SIG_EN != 0`，中断线就保持高电平。这就是为什么 ISR 里必须 mask SIG_EN（防止 ISR 返回后立即再次触发）。

### 2.3 关键状态位

Normal Interrupt Status 中我们关心的位：

| 位 | 掩码 | 含义 |
|----|------|------|
| bit 0 | `NORM_INT_CMD_COMPLETE` | 命令完成（CMD52/CMD53 命令阶段结束） |
| bit 1 | `NORM_INT_XFER_COMPLETE` | **传输完成**（数据阶段结束，FIFO→芯片的 DMA 完成） |
| bit 4 | `NORM_INT_BUF_WR_READY` | 缓冲区可写（PIO 写入 FIFO 前检查） |
| bit 5 | `NORM_INT_BUF_RD_READY` | 缓冲区可读（PIO 从 FIFO 读取前检查） |
| bit 8 | `NORM_INT_CARD_INT` | **卡中断**（WiFi 芯片通知 CPU："我有数据"） |
| bit 15 | `NORM_INT_ERROR` | 汇总错误标志（任一 Error 位被置位） |

---

## 3. 驱动架构

### 3.1 分层结构

```
┌───────────────────────────────────────────────┐
│  StarryOS 网络栈 (ax_net / smoltcp)            │
│  "发这个以太网帧"    "有数据来了吗？"            │
└──────────────────┬────────────────────────────┘
                   │ rd_net::Interface trait
                   ▼
┌───────────────────────────────────────────────┐
│  aic8800 驱动 (components/aic8800)             │
│  - 固件通信协议 (LMAC 消息)                     │
│  - WiFi STA/AP 控制 (连接/扫描/密钥)            │
│  - TX/RX 数据路径 (以太网 ↔ 802.11)            │
│  - 后台线程 (wifi-tx, wifi-rx, wifi-ap)        │
│  通过 trait WifiRuntime 注入 OS 能力           │
└──────────────────┬────────────────────────────┘
                   │ SdioHost trait (sdio-host crate)
                   ▼
┌───────────────────────────────────────────────┐
│  sdhci-cv1800 驱动 (components/sdhci-cv1800)   │
│  - SDHCI 寄存器操作 (CMD52/CMD53/PIO)          │
│  - SDIO 卡枚举 (CMD5/CMD3/CMD7)               │
│  - 中断处理 (ISR)                              │
│  - 时钟/电源/总线宽度配置                       │
│  通过 trait SdhciDelay 注入 OS 能力            │
└──────────────────┬────────────────────────────┘
                   │ MMIO (read_volatile / write_volatile)
                   ▼
┌───────────────────────────────────────────────┐
│  SG2002 SDHCI 控制器硬件                       │
│  基地址 0x04320000                             │
└──────────────────┬────────────────────────────┘
                   │ SDIO 总线
                   ▼
┌───────────────────────────────────────────────┐
│  AIC8800 WiFi 芯片 + 固件                      │
└───────────────────────────────────────────────┘
```

### 3.2 两个 trait：依赖注入

驱动层和 OS 层通过 trait 解耦。驱动不直接依赖 `ax_task::sleep()` 或 `ax_hal`，而是定义 trait 要求 OS 提供能力。

**SdhciDelay**（SDHCI 控制器的 OS 需求）：
```rust
pub trait SdhciDelay {
    fn delay_ms(&self, ms: u64);       // 阻塞延迟
    fn yield_now(&self);                // 让出 CPU
    fn block_timeout(&self, timeout_ms: u64) -> bool;  // 阻塞等待中断
}
```

**WifiRuntime**（aic8800 驱动的 OS 需求）：
```rust
pub trait WifiRuntime {
    fn now_nanos(&self) -> u64;         // 单调时钟
    fn sleep_ms(&self, ms: u64);       // 阻塞延迟
    fn yield_now(&self);                // 让出 CPU
    fn spawn_poll_task(&self, name: &str, poll: Box<SendPollFn>);  // 启动后台任务
    fn block_until(&self, timeout: Option<u64>, poll: &mut PollFn) -> Result<(), TimedOut>;
}
```

OS 胶水层 `wifi_glue.rs` 实现这两个 trait，在 `install_runtime()` 中注入。

### 3.3 WifiBus 结构

`WifiBus` 是驱动运行时的中心数据结构，包含五个子结构：

```
WifiBus ── transport    SdioTransport（SDIO 传输层，封装所有 CMD52/CMD53 操作）
        ├─ state        BusState（Up/Down）
        ├─ conn         ConnectionState（vif_idx, sta_idx, MAC, 连接状态）
        ├─ cmd          CmdState（命令待发队列、CFM 等待、响应队列）
        ├─ rx           RxState（数据接收队列、EAPOL 队列、TX CFM 队列、IRQ waker）
        ├─ tx           TxState（数据发送队列、pktcnt、wake_pollset）
        └─ ap           ApState（关联请求队列、已注册 STA 列表、控制端口状态）
```

每个子系统有独立的队列和唤醒机制，通过 `PollSet`（类似 epoll 的等待集合）或 `AtomicWaker` 协调线程间通信。

### 3.4 后台线程

驱动启动三个长期运行的后台任务，每个跑在自己的线程上：

| 线程 | 入口 | 职责 |
|------|------|------|
| **wifi-tx** | `thread/tx.rs::start()` | 从 TX 队列取帧，写入 WiFi 芯片（数据帧 + 管理帧 + CMD） |
| **wifi-rx** | `thread/rx.rs::start()` | 响应 CARD_INT，从 WiFi 芯片读数据，分发给各队列 |
| **wifi-ap** | `thread/ap.rs::start()` | 处理 STA 关联/去关联请求，控制端口管理（仅 AP 模式） |

---

## 4. 中断系统

### 4.1 两种中断

整个 WiFi 子系统涉及两种中断源：

| 中断 | 来源 | 触发条件 | 处理者 |
|------|------|----------|--------|
| **CARD_INT** | AIC8800 芯片通过 SDIO 总线 | 芯片有数据要传给 CPU（收到无线帧、固件响应等） | aic8800 的 `sdio1_irq_handler` |
| **XFER_COMPLETE** | SDHCI 控制器自身 | SDIO 总线上的数据传输完成（CMD53 的数据阶段结束） | 唤醒阻塞在 `poll_int_status` 的任务 |

### 4.2 CARD_INT 完整流程

这是"芯片通知 CPU 有数据"的路径，由 **ISR → RX 线程** 接力完成：

```
1. AIC8800 芯片收到无线帧
   → 通过 SDIO 总线向控制器发送"卡中断"信号

2. SDHCI 控制器：
   → 置位 INT_STATUS 的 CARD_INT 位（bit 8）
   → 因为 SIG_EN 中 CARD_INT=1，拉高中断线
   → PLIC 路由 IRQ #38 → CPU

3. CPU 跳转到 sdhci_irq_handler（irq.rs）：
   → 读 INT_STATUS，看到 CARD_INT=1
   → mask CARD_INT 信号（写 SIG_EN，清 CARD_INT 位）
     （防止 ISR 返回后电平触发立即重入）
   → 调用 card_irq_callback = sdio1_irq_handler

4. sdio1_irq_handler（bus.rs）：
   → 再 mask 一次 CARD_INT（通过 SDHCI 控制器的另一个路径）
   → 设置 bus.rx.irq_pending = true
   → bus.rx.irq_waker.wake()  ← 唤醒 RX 线程

5. RX 线程（wifi-rx）醒来：
   → 检查 irq_pending 标志
   → 读 SDIO 寄存器获取待收长度
   → 通过 PIO 从芯片 FIFO 读出数据
   → 解析帧类型（数据帧 / EAPOL / CFM / 固件响应）
   → 入队到对应队列，唤醒对应消费者
   → unmask CARD_INT 信号
```

### 4.3 XFER_COMPLETE 中断流程（新增）

这是"SDIO 传输完成"的路径，用于替代原来的 `yield_now()` 等待：

```
1. TX 线程调用 write_fifo → poll_int_status(NORM_INT_XFER_COMPLETE)
   → Phase 1 自旋 1000 次，没等到
   → 进入 Phase 2

2. Phase 2：
   → unmask_xfer_complete_signal()
     （向 SIG_EN 寄存器写入：原有的 CARD_INT 位 + XFER_COMPLETE 位 = 1）
   → block_timeout(10ms)
     → WaitQueue::wait_timeout
     → 把 TX 线程标记为 BLOCKED，放入等待队列
     → 调度器切换到其他就绪任务

3. SDIO 硬件传输完成（~200µs）
   → 置位 INT_STATUS 的 XFER_COMPLETE 位（bit 1）
   → 因为 SIG_EN 中 XFER_COMPLETE=1，拉高中断线
   → PLIC → CPU

4. CPU 跳转到 sdhci_irq_handler：
   → 读 INT_STATUS，看到 XFER_COMPLETE=1
   → mask XFER_COMPLETE 信号（写 SIG_EN，清 XFER_COMPLETE 位）
   → 不碰 INT_STATUS 的 XFER_COMPLETE 位（让它保持 1，留给任务读）
   → 调用 pio_wake_callback = sdhci_pio_wake_callback

5. sdhci_pio_wake_callback（wifi_glue.rs）：
   → SDHCI_PIO_WQ.notify_one_from_irq()
   → 从等待队列取出 TX 线程
   → 标记为 READY
   → ISR 返回

6. 调度器看到 TX 线程就绪 → 调度它

7. TX 线程从 block_timeout 返回（返回值 false = 被中断唤醒）
   → check_int_status()
   → 读 INT_STATUS，XFER_COMPLETE=1 ✓
   → W1C 写 1 清除该位
   → 返回 Ok(())
```

### 4.4 中断嵌套安全性

在单核 RISC-V 上，`SendNoIrq` 锁在持有时禁用本地中断。因此：

- 任务持 `WaitQueue` 锁期间，ISR 不可能抢占（中断被禁用）
- ISR 内调用 `notify_one_from_irq` 时，锁一定是释放状态（任务还未持有，或已释放）
- 不存在死锁风险

这是单核系统的天然串行化保证。SMP 上需要额外的 fencing。

---

## 5. TX 数据通路

### 5.1 从网络栈到 WiFi 芯片

```
应用层 (iperf3 / akars)
  │ send()
  ▼
smoltcp TCP 栈
  │ 构造以太网帧
  ▼
ax_net → wlan0 (rd_net::Interface::transmit)
  │ 入队: bus.tx.queue.push_back(frame)
  │ bus.tx.pktcnt += 1
  │ bus.tx.wake_pollset.wake()  ← 唤醒 TX 线程
  ▼
wifi-tx 线程醒来
  │ process_data_tx(bus)
  ▼
check_data_flow_control()
  │ 读 SDIO 流控寄存器
  │ 固件有多余 buffer → 继续
  │ 缓冲区满 → yield 重试（最多 50 次）
  ▼
send_single_data_frame()
  │ 以太网帧 → 802.3 封装（加 tail、对齐）
  │ 加 SDIO 头部（type=DATA）
  ▼
transport.write_fifo(func=1, addr=WR_FIFO, &buf)
  │ sdio.lock().write_fifo()
  ▼
sdhci-cv1800: CviSdhci::write_fifo()
  │ cmd53_write_fixed()
  ├─ cmd53_xfer()           ← 发 CMD53 命令
  │   ├─ wait_data_idle()   ← 等 CMD+DAT 线空闲
  │   ├─ 写 BLOCK_SIZE, BLOCK_COUNT
  │   ├─ 写 ARGUMENT, 写 COMMAND（原子写入）
  │   └─ wait_cmd_complete()← 等命令响应
  ├─ pio_write()            ← PIO 写入 FIFO
  │   └─ for each block:
  │       wait_buffer_write_ready()  ← 等 FIFO 可写
  │       for each word:
  │         写 BUFFER 寄存器（32-bit）
  └─ wait_transfer_complete()       ← ★ 等硬件把 FIFO 数据发给芯片
      └─ poll_int_status(XFER_COMPLETE)
         Phase 1: 自旋 1000 次 (~50µs)
         Phase 2: block_timeout(10ms) → ISR 唤醒
```

### 5.2 瓶颈在哪里

`wait_transfer_complete` 是整条 TX 路径中最长的等待点。

- `wait_buffer_write_ready`：CPU 往控制器 FIFO 写数据，FIFO 在控制器内部，微秒级完成
- `wait_transfer_complete`：控制器把 FIFO 数据通过 SDIO 总线发给芯片，**~200µs**

`wait_transfer_complete` 内部调用 `poll_int_status(NORM_INT_XFER_COMPLETE)`，轮询 INT_STATUS 寄存器等待 bit 1 被硬件置位。

---

## 6. RX 数据通路

### 6.1 从 WiFi 芯片到网络栈

```
AIC8800 芯片收到无线 802.11 帧
  │ 芯片固件处理: 解密、去 802.11 头、重组
  │ 数据放入芯片内部 FIFO
  ▼
拉 CARD_INT 信号
  │
  ▼
SDHCI 控制器: 置 INT_STATUS[CARD_INT]=1 → 中断 → ISR
  │
  ▼
sdio1_irq_handler → irq_pending=true → rx.irq_waker.wake()
  │
  ▼
wifi-rx 线程醒来
  │ process_rx(bus)
  │ 读 block_cnt_reg / bytemode_len_reg 获取长度
  ├─ transport.read_fifo(func, RD_FIFO, &mut buf)
  │   └─ sdhci-cv1800: read_fifo()
  │       ├─ cmd53_xfer(READ)
  │       ├─ pio_read()  ← PIO 从 FIFO 读出数据
  │       └─ wait_transfer_complete()
  ▼
解析帧头:
  ├─ type=CFG_CMD_RSP  → rsp_queue（命令响应，唤醒 cmd.rsp_pollset）
  ├─ type=CFG_DATA_CFM → tx_cfm_queue（发送确认，唤醒 tx_cfm_pollset）
  ├─ type=EAPOL        → eapol_queue（加密握手，唤醒 eapol_pollset）
  ├─ type=DATA         → data_queue（数据帧）
  │   └─ 802.3 → 以太网帧
  │     设置 RX_DATA_PENDING=true
  │     调用 rx_data_callback → ax_net::wake_net_task_irq
  └─ type=CFG_PRINT     → 固件日志
  ▼
网络栈 poll 驱动 → smoltcp 处理 → 交付给应用层
```

### 6.2 RX 线程的轮询兜底（kicker）

WiFi 唤醒机制依赖 PollSet（边沿触发，非 sticky）。如果 ISR 的 `wake()` 发生在 RX 线程注册 waker 的窗口之外，唤醒事件会永远丢失。

因此 RX 线程启动一个 10ms 周期的 kicker 兜底任务：即使 CARD_INT 唤醒丢失，最多 10ms 后 kicker 也会把 RX 线程踢醒。这和 TX 路径的 `block_timeout(10ms)` 超时兜底是同一思路。

---

## 7. 中断方案的改动总览

### 7.1 改动前（yield 方案）

```
poll_int_status Phase 2:
  for i in 0..200_000 {
      读 INT_STATUS → 到位？返回
      否则 yield_now()  ← 把 CPU 给其他任务
  }
```

`yield_now()` 的问题：它把当前任务放回就绪队列尾部，CPU 被其他任务占满 50ms 时间片才能回来。硬件 200µs 就完成了，但任务要等 48ms。

### 7.2 改动后（中断方案）

```
poll_int_status Phase 2:
  for i in 0..20 {
      读 INT_STATUS → 到位？返回
      [仅 XFER_COMPLETE] unmask 中断信号
      block_timeout(10ms)  ← 阻塞自己，而不是 yield
      读 INT_STATUS → 到位？返回
  }
```

`block_timeout(10ms)` 把任务标记为 BLOCKED（从就绪队列移除）。调度器不调度它，直到：

- **正常路径**：硬件完成 → ISR 唤醒任务 → 任务从 block_timeout 返回 → 读 INT_STATUS → XFER_COMPLETE=1 → 返回（总延迟 ~200µs + 调度开销）
- **超时路径**：10ms 后定时器自动唤醒 → 读 INT_STATUS → 没到位 → 下一轮循环
- **误唤醒路径**：其他中断触发，ISR 间接导致任务被唤醒 → 读 INT_STATUS → 没到位 → 下一轮循环（无害）

### 7.3 涉及的文件

| 文件 | 改动内容 | 改动性质 |
|------|----------|----------|
| `runtime.rs` | trait 新增 `block_timeout` 方法 | 接口扩展 |
| `irq.rs` | ISR 新增 XFER_COMPLETE 处理 + 唤醒回调机制 | 新增功能 |
| `lib.rs` | `poll_int_status` Phase 2 从 yield 改为中断驱动阻塞 | 核心改动 |
| `wifi_glue.rs` | 用 WaitQueue 实现 `block_timeout` + 注册回调 | OS 胶水实现 |

- **未改动**：aic8800 驱动代码完全不变；SDIO 枚举/初始化/CMD52/CMD53/PIO 逻辑完全不变
- **Phase 1（1000 次自旋）不变**：仍是快速路径，~50µs 内命中
