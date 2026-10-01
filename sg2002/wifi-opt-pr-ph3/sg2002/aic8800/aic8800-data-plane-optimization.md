# AIC8800 数据面异步化分析与优化方案

分析对象：`drivers/net/aic8800`（SDIO WiFi，SG2002/CV181x）
基线：dev 分支 `2f5347b14`

本文只记录已核实的结论，每条都带 `文件:行号`。推断和未确认项集中在第 7 节，不混入结论。

> 第 6 节「优化方案」的优先级已被 `aic8800-tx-throughput-analysis.md` 取代：实测吞吐表明瓶颈是每帧的串行往返，不是拷贝次数。本文其余部分（路径逐跳、短路条件、分层对比）仍然有效。

---

## 1. 结论摘要

1. aic8800 驱动**不含任何 Rust 异步原语**（无 `async fn` / `.await` / `Future` / `Waker` / `poll_fn`），也不依赖 `ax_task` / `ax_sync` / `axruntime`。它是一套事件驱动的串行状态机。
2. 它的"异步"由**进度返回值**表达：一次推进返回 `Ready` 或 `Wait(Interrupt | InterruptUntil | RetryAt)`，执行流从不挂起。
3. 数据面的主要开销不是"没有异步"，而是**每帧多次堆分配与拷贝，且全部串在硬件事务之间的关键路径上**。
4. 优化不需要引入 `async`/`Future`，落点是减少拷贝次数、把可离线的工作移出关键路径。改动全部可以限制在 `drivers/net/aic8800` 内部，不触碰与 SD 卡共用的 `drivers/blk` 契约层。

---

## 2. 驱动现状

### 2.1 无 Rust 异步原语

`grep` `drivers/net/aic8800/src/` 的 `async fn | .await | Future | Waker | poll_fn` → 零匹配。
`Cargo.toml` 与源码均无 `ax_task` / `ax_sync` / `axruntime` 引用。

设计文档 `docs/design/unified-sdio-aic8800.md:27-29` 记录了取舍：

> AIC 比较过同步包装器、驱动自建线程和 owner 驱动状态机。同步包装器会隐藏取消/超时，驱动线程会绑定 OS；因此核心与 adapter 均只返回有限步骤进度，由外层 owner 决定等待策略。

### 2.2 四层架构

设计文档 `docs/design/unified-sdio-aic8800.md` 的职责表：

| 层 | 位置 | 职责 |
| --- | --- | --- |
| Driver Core | `drivers/net/aic8800/src/device`、`tx.rs`、`rx.rs`、`protocol.rs` | 寄存器语义、固件/命令/RX/TX 状态机 |
| Capability Adapter | `drivers/net/aic8800/src/rdif`（`rdif` feature） | 转换 `SdioCard`、拥有型 DMA、RDIF token、IRQ 快照 |
| OS Glue | `drivers/ax-driver` | FDT、MMIO、pinmux/reset/clock、设备注册 |
| Runtime | `net/ax-net`、`axruntime` | 固定 CPU owner、定时唤醒、IRQ 注册/亲和性 |

分层约束：Driver Core 不依赖 RDIF；Adapter 不依赖 `ax_task` / `axruntime`，不创建线程、不休眠、不自旋。

### 2.3 进度契约

三层进度类型逐级映射：

| 层 | 类型 | 位置 |
| --- | --- | --- |
| Driver Core | `AicAction`（`WaitForInterrupt` / `WaitForInterruptUntil` / `RetryAt` / `SubmitSdio` / `AbortSdio` / `Event` / `Idle`） | `src/device/progress.rs` |
| Capability Adapter | `OwnerProgress` / `OwnerWait` | `src/rdif/owner/progress.rs:27-38` |
| sdmmc 协议 | `HostProgressWait` / `OperationProgress` | `drivers/blk/sdmmc-protocol` |

`OwnerWait` 三态：

```rust
pub(crate) enum OwnerWait {
    Interrupt,              // 只等下一次设备中断
    InterruptUntil(u64),    // 等中断或绝对超时（mailbox confirmation 用）
    RetryAt(u64),           // 纯定时等待
}
```

设备核心通过 `AicDevice::advance(AicInput) -> AicAction` 推进（`src/device/progress.rs:21`），只返回**一个**外部可见转换。

---

## 3. TX 数据面

### 3.1 完整路径

```
T0  协议层 AicTxQueue::submit(DmaBuffer)              rdif/device/queues.rs:70
      ↓ SPSC 环 tx_submit（容量 = QueueConfig.ring_size）

T1  output.rs::take_tx_frame()                        rdif/owner/output.rs:61-70
      buffer.complete_for_cpu(len)
      buffer.read_with_cpu(len, |b| b.to_vec())       ← 拷贝 #1 + 堆分配
      tx_tokens.push_back((token, buffer))            ← 原 DMA buffer 留待回收
    → AicInputEvent::Tx { token, frame }
    → TxState::enqueue(token, frame)                  tx.rs:26   入队

T2  advance(tick) → drive_ready()                     device/data_plane.rs:22
      prepare_next_transmit()                         device/data_plane.rs:420
        take_wire_frame() → ethernet_tx_frame()       tx.rs:37 / protocol.rs:154
          vec![0; final_len] + 搬 payload + 写头      protocol.rs:174   ← 拷贝 #2
        active_tx = Some(ActiveTx { wire_frame })
      emit(TransmitFlow, read_byte(flow_control))     ← 同一调用内立即发 CMD52
      io.pending = Some(...)                          progress.rs:184

T3  等 CMD52 返回
      advance(...) → io.pending.is_some() → WaitForInterrupt

T4  advance(Sdio(completion)) → consume_transmit_flow()   device/data_plane.rs:373
      credits = flow_credits(...)                     registers.rs:135-141
      ├ credits <= DATA_TX_RESERVED_CREDITS(2) → active.retry_at = now + IO_RETRY(1ms)
      │               后续 WaitForInterruptUntil(deadline)
      └ credits > 2  → active.wire_frame.clone()      device/data_plane.rs:388  ← 拷贝 #3
                       emit(TransmitData, write_fifo(..., frame))

T5  等 CMD53 返回

T6  advance(Sdio(completion)) → consume_transmit_data()   device/data_plane.rs:396
      active_tx.take()，wire_frame 被丢弃
      push_event(TransmitComplete(token))

T7  advance(tick) → pop_event() → AicAction::Event(TransmitComplete(token))
```

结论：一帧 TX = **构造 wire frame → CMD52 读 credit → clone → CMD53 写**，四步严格串行。
**wire frame 的构造发生在 CMD52 之前**（同一个 `drive_ready()` 调用内，`:443` 构造、`:71` 紧接发 CMD52），credit 返回后做的是 `clone()`。

常量：`DATA_TX_RESERVED_CREDITS = 2`（`data_plane.rs:17`）、`IO_RETRY = 1ms`（`:14`）、`TX_CAPACITY = 128`（`tx.rs:8`）、`INTERNAL_TX_CAPACITY = 2`（`:18`）。

### 3.2 `advance()` 的短路条件

TX 要推进到下一帧，必须穿过四层共 17 个提前返回。

#### 第一层 `advance()`（`device/progress.rs:21-58`）

| # | 条件 | 返回 |
| --- | --- | --- |
| ① | 时间回退 | `fail(error)` |
| ② | `consume_input` 出错 | `fail(error)` |
| ③ | 事件队列非空（`pop_event`） | `AicAction::Event` |
| ④ | `cancel_pending && io.pending.is_some()` | `AbortSdio` |
| ⑤ | `io.pending.is_some()` | `WaitForInterrupt` |
| ⑥ | `io.next` 有预备请求 | `emit(...)` |
| ⑦ | `lifecycle.retry_at` 未到 | `RetryAt(deadline)` |

#### 第二层 `drive_ready()`（`device/data_plane.rs:22-77`）

按优先级排列：**mailbox > 优先事件 > RX scan > TX**

| # | 条件 | 返回 |
| --- | --- | --- |
| ⑧ | mailbox 超时 | `drive_mailbox` |
| ⑨ | mailbox 进行中且等接收 | `drive_receive_scan` |
| ⑩ | mailbox 进行中 | `drive_mailbox` |
| ⑪ | 控制命令队列非空 | `drive_mailbox` |
| ⑫ | 优先事件（非 `Receive`） | `AicAction::Event` |
| ⑬ | RX scan 激活 | RX 的 IO |

注：⑫ 的 `take_priority_event()`（`:79`）只在 mailbox 完成路径可达 —— `mailbox.rs:344-349` 的 `drive_startup_or_ready()` 在 `push_event(ControlComplete)`（`:339`）之后直接调 `drive_ready()`，绕过了 `advance()` 的 `pop_event`。经 `advance()` 进入时事件队列必为空。

#### 第三层 `prepare_next_transmit()`（`device/data_plane.rs:420-463`）

| # | 条件 | 后果 |
| --- | --- | --- |
| ⑭ | `active_tx.is_some()` | 直接 return（单帧在飞） |
| ⑮ | `link.tx_indices()` 为 `None` | 无 TX |
| ⑯ | TX 队列空 | 无 TX |

#### 第四层 TX 分支内（`device/data_plane.rs:62-75`）

| # | 条件 | 返回 |
| --- | --- | --- |
| ⑰ | `active.retry_at` 未到 | `WaitForInterruptUntil(deadline)` |

### 3.3 提前准备情况

| 环节 | 是否提前 | 位置 |
| --- | --- | --- |
| DmaBuffer → 堆上裸帧 | 是 | `rdif/owner/output.rs:65`，协议层提交时立即做 |
| 入队积压 | 是 | `tx.rs:26`，容量 128 |
| 裸帧 → wire frame | **否** | `protocol.rs:174`，在 `prepare_next_transmit` |
| wire frame → CMD53 的副本 | **否** | `data_plane.rs:388`，credit 通过后才做 |
| 下一帧的 wire frame 构造 | **否** | 被 ⑭ 挡住 |

三处等待窗口（等 CMD52、等 CMD53、credit 退避 1ms）中，**没有任何 TX 准备工作在执行**：

- 等 CMD52 / CMD53 时，`io.pending.is_some()` 在 ⑤ 处短路，`drive_ready()` 根本不被调用
- credit 退避时，`active_tx` 已占用，⑭ 短路

⑤ 本身是正确的（SDIO 总线同时只能有一个事务在飞），问题在于它**同时挡住了纯 CPU 工作**。

---

## 4. RX 数据面

### 4.1 完整路径

```
R1  CARD_INT → request_receive_scan()                 device/data_plane.rs:88
R2  drive_receive_scan()                              device/data_plane.rs:109-123
      emit(ReceiveCount(path), read_byte(block_count))  ← CMD52 读块数
R3  consume_receive_count()                           device/data_plane.rs:125-169
      ReceiveLength::Empty        → 下一条路径
      ReceiveLength::OtherInterrupt → ReceiveOtherAck 软中断应答
      ReceiveLength::Blocks(n)    → read_fifo(fn, read_fifo, n * BLOCK_SIZE)
      ReceiveLength::ByteMode     → ReceiveByteLength（DC 走这条，多一次往返）
      ↑ 长度此刻才知道
R4  consume_receive_data()                            device/data_plane.rs:229
      receive_data = expect_data(response)            ← FIFO 读缓冲
      parse_fifo(&receive_data, confirmation_id)      rx.rs:109
R5  逐个消费 ParsedFrame                              device/data_plane.rs:258-274
      ParsedFrame::Data { frame, decryption_status }
        → decapsulate_data_frames(&frame, status)     :537
        → EAPOL 走 consume_eapol，否则 push_event(Receive(frame))
```

### 4.2 每帧的分配与拷贝

| 步 | 位置 | 操作 |
| --- | --- | --- |
| ① | SDIO 层 | `SdioResponse::Data(Vec<u8>)` FIFO 读缓冲，1 次分配 |
| ② | `rx.rs:233` | `bytes[offset+60..offset+aggregate_len].to_vec()` MPDU 载荷，1 次分配 + 拷贝 |
| ③ | `data_plane.rs:578` | `vec![frame]` —— 仅为统一 A-MSDU / 非 A-MSDU 返回类型的包装 |
| ④ | `data_plane.rs:621` | `ethernet_from_llc` → `Vec::with_capacity(...)` **最终以太网帧** |
| ⑤ | `rdif/owner/output.rs:192-198` | 从 `rx_submit` 取预分配 `DmaBuffer`，`copy_from_slice` |

一帧 RX ≈ **4 次堆分配 + 4 次拷贝**。

### 4.3 两个已核实的性质

**性质 A：② 的拷贝是纯中间的。**
`consume_receive_data` 在**同一次调用内**就完全消费了 `ParsedFrame::Data`（`data_plane.rs:258-274`：`decapsulate_data_frames` → `push_event`），所以数据帧可以直接借用 FIFO buffer。

`parse_fifo` 的文档注释是 "Parses a FIFO aggregation without retaining aliases into the transfer buffer"（`rx.rs:107`）。这个约束对**控制帧是必要的** —— `Confirmation` / `Indication` 的 `payload` 会被 mailbox 状态机长期持有；但对**数据帧不必要**。

**性质 B：RX 侧已有预分配 DMA buffer 池。**
`QueueOwnerPorts`（`rdif/device/queues.rs:11-17`）：

```rust
pub(crate) struct QueueOwnerPorts {
    pub(crate) tx_submit: HeapCons<DmaBuffer>,
    pub(crate) tx_complete: HeapProd<DmaBuffer>,
    pub(crate) rx_submit: HeapCons<DmaBuffer>,   // 协议层预提交的空 buffer 环
    pub(crate) rx_complete: HeapProd<RxCompletion>,
    pub(crate) rx_frame_size: usize,
}
```

但它只在管线末端（`publish_rx`，`output.rs:192`）被使用，且是作为**拷贝目标**。

**性质 C：底层 SDIO 协议已支持拥有型 DMA，但 AIC adapter 未使用。**
`drivers/blk/sdmmc-protocol/src/sdio/io/transfer.rs:26` 定义了 `SdioDmaTransferRequest<H>`，构造时接收 `PreparedDma`（`:145`、`:164`）。
`drivers/blk/sdmmc-protocol/src/sdio/host.rs:148-152` 注释：

> Returns the DMA capability owned by this physical host. Protocol initialization uses it for CPU-owned scratch DMA. Production block I/O already arrives as `PreparedDma`.

而 AIC adapter 走的是 `SdioRequestKind::Read { length }`（`src/device/model.rs:129-135`），只有长度，由协议层分配 `Vec<u8>`。

---

## 5. 与其他实现的对比

### 5.1 块设备异步运行时（PR #2349，commit `ac8eaf7bb`）

PR 标题 `feat(ax-fs-ng): add async block request runtime`。

**不是 future executor，`block_on` 在这些路径中不出现。** 真实结构：

- 每个硬件队列一个 pinned kthread，线程名 `blk-hctx/{id}`（`fs/ax-fs-ng/src/block/runtime/hctx/mod.rs:198`），由 `BlockRuntimeOps::spawn_pinned` 创建（trait 定义 `fs/ax-fs-ng/src/os/task.rs:60-67`，实现 `os/arceos/modules/axruntime/src/fs/block.rs:149`）
- worker 循环 `run_hctx`（`hctx/mod.rs:554`）在算完 deadline 后阻塞（`:667` `wait_timeout` / `:669` `wait`）
- 硬中断只做确认与 latch：`BlockIrqAction::run()`（`fs/ax-fs-ng/src/block/runtime/irq.rs:100`）的文档注释明确 "performs no allocation, queue drain, DMA copy, registry lookup, filesystem access, or business-task wakeup"
- Future 只出现在**调用方一侧**，管两件事：新提交的 admission（`lifecycle/submission.rs:131` `submit_owned_async`）和 completion receiver（`completion.rs:123`，`poll_fn` + `AtomicWaker`），由调用方任务 poll

PR 正文明确保留项：

> 保留 `OwnedRequest`、`PreparedDma`、`validate_owned_request()`、现有 bounded channel、hctx maintenance thread、IRQ 通知链路和 DMA 所有权模型。

> 不改变同步 completion/group API 的行为。

即：**线程未被去掉，新增的是"等待"的第二种实现**（从阻塞 `wait()` 改为注册 waker）。

对 aic8800 的映射：

| #2349 | aic8800 |
| --- | --- |
| hctx maintenance thread（固定 CPU） | `net/ax-net/src/queue_runtime` 的固定 CPU owner |
| `CompletionCell` + `AtomicWaker` | IRQ latch + `ax_task::sync::WaitQueue` |
| `submit_owned_async`（调用方 future 等待） | `submit_one_tx`（owner 主动轮询队列） |

唯一实质差别：`submit_owned_async` 让调用方能把"等这个请求"变成可挂起对象，从而在同一任务内交替推进多个请求。aic8800 的 owner 是唯一驱动者。

### 5.2 共用 SG2002 SDIO 契约的使用者

SG2002 上只有两个 SD/SDIO 控制器，各挂一个设备（DTB `os/StarryOS/configs/board/aka-00-sg2002.dtb` 与 `licheerv-nano-sg2002.dtb`）：

| DTB 节点 | compatible | 设备 | 适配层 |
| --- | --- | --- | --- |
| `cv-sd@4310000` | `cvitek,cv181x-sd` | SD 卡块设备 | `drivers/ax-driver/src/block/cvsd.rs`（`DEVICE_NAME = "cvsd"`，`:32`） |
| `wifi-sd@4320000` | `cvitek,cv181x-sdio` | AIC8800 SDIO WiFi | `drivers/ax-driver/src/net/aic8800/mod.rs:47` |

该结论由回归测试锁定：`drivers/ax-driver/src/cv181x/mod.rs:192` `repository_sg2002_dtbs_describe_every_cv181x_sd_and_sdio_region`。

**共用的（全部同步）：**
`drivers/blk` 下 7 个 SD/MMC/SDIO 相关软件包 —— `sdmmc-host`、`sdmmc-protocol`、`sdhci-host`、`cv181x-sdhci`、`dwmmc-host`、`phytium-mci-host`、`starfive-jh7110-dwmmc`。全部无 `async fn` / `.await` / `Future`。

同步模型显式定义在 `drivers/blk/sdmmc-host/src/host/mod.rs:63`：

```rust
pub enum ProgressCause { Submitted, AcknowledgedIrq, RegisterRetry }
```

驱动被逐步"拉"，调用方提供 cause —— 与 aic8800 的 `AicAction` 是同一模式。

`drivers/ax-driver/src/sdhci_runtime.rs` 仅 19 行，只安装 `HostTimer` 时钟适配，不驱动队列、不等待、不涉及调度。

**不共用的（各自一套 runtime）：**

| | SD 卡 | AIC8800 WiFi |
| --- | --- | --- |
| IRQ 线 | 0x24 | 0x26 |
| runtime | `ax-fs-ng` block runtime | `ax-net` queue runtime |
| 线程 | `blk-hctx/{id}` | `net-queue-cpu{cpu}`（`net/ax-net/src/queue_runtime/mod.rs:558`） |
| RDIF 契约 | `rdif_block` | `rdif_eth`（`drivers/interface/rdif-eth/src/lib.rs:595` `NetPollGroupParts`、`:609` `NetDeviceParts`） |

唯一直接使用 `WaitQueue` 的位置在 WiFi 控制路径：`WifiCommandCompletion::wait()`（`net/ax-net/src/queue_runtime/mod.rs:73-80`）→ `self.wait.wait_until(...)`。

**对改动的含义：** aic8800 侧改动不影响 SD 卡路径；反之改动 `sdmmc-host` / `sdhci-host` / `cv181x-sdhci` / `sdmmc-protocol` 会同时影响 SD 卡。

---

## 6. 优化方案

### 不变量

1. `rdif_eth` 的 `NetDeviceParts` / `NetPollGroupParts` / `QueueFramePort` 接口不动
2. `AicAction` / `OwnerProgress` / `OwnerWait` 三态语义不动
3. IRQ 时序不动：`CardIrqWait` 状态机、`rearm_completion_irq_and_check()` 的 completion-before-card 顺序、`pending_irq_retry` 的 retry 语义
4. 分层不动：Driver Core 不依赖 RDIF / `DmaBuffer`；Adapter 不依赖 `ax_task` / `axruntime`
5. 不修改 `drivers/blk` 下与 SD 卡共用的契约层

### 阶段 1（TX）：消掉关键路径上的全帧 `clone()`

**位置**：`src/device/data_plane.rs:388` `let frame = active.wire_frame.clone();`

**已核实的三条路径：**

- `consume_transmit_data`（`:396-402`）只 `active_tx.take()`，不读 `wire_frame` —— 成功路径上 clone 完全浪费
- `fail()`（`src/device/progress.rs:224-228`）只保留 token 推 `TransmitComplete`，`wire_frame` 直接丢弃
- credit 退避（`:384-387`）在 emit **之前**，此时仍需保留 `wire_frame`，不能无条件 `take()`

**改法**：`ActiveTx.wire_frame` 改为 `Option<Vec<u8>>`；`consume_transmit_flow` 在 credit 通过时 `take()` 交给 `write_fifo`，退避时保持原样。

**待判定（见第 7 节 Q1）**：`finish_cancel()`（`src/device/progress.rs:204-215`）不清 `data.active_tx`。若 abort 后 `state` 仍为 `Ready`，下一轮 `drive_ready` 会看到残留的 `active_tx` 并重新 emit `TransmitFlow`。该行为改动前后一致，但 `Option` 化后若在 abort 路径已被 take 空，重发会发出空帧。

**收益**：每帧省 1 次堆分配 + 1 次全帧拷贝。

### 阶段 2（TX）：把 wire frame 构造移出关键路径

**现状**：`prepare_next_transmit()`（`:420`）只在 `drive_ready()`（`:61`）末尾被调用；`advance()` 在 `io.pending.is_some()` 时于 `progress.rs:40-42` 短路，导致等待窗口内零准备。

**改法**：在 `advance()` 的 `io.pending` 短路之前插入纯 CPU 准备步骤：

```rust
// src/device/progress.rs
if let Some(event) = self.data.pop_event() { return AicAction::Event(event); }   // ③
if self.lifecycle.cancel_pending && ... { return AicAction::AbortSdio { ... }; } // ④
self.prepare_offline();                                                          // 新增
if self.io.pending.is_some() { return AicAction::WaitForInterrupt; }              // ⑤
```

`prepare_offline()`：在 `active_tx.is_none()` 且无更高优先级工作时，把下一帧 wire frame 预构造进 `staged: Option<(TxToken, Vec<u8>)>`。

**约束**：
- 纯 CPU：不碰 `io`、不碰 `irq_latch`、不返回 `AicAction`
- 幂等（`advance()` 会被反复调用）
- 必须复刻 `drive_ready` 的优先级准入（mailbox > 优先事件 > RX scan > TX），否则破坏 RX 优先语义

**风险**：第三条是本阶段最易出错处。

### 阶段 3（TX）：多帧流水

**阻塞项**（见第 7 节 Q2、Q3）：
1. firmware 是否接受一次 CMD53 写多个 hostdesc
2. `flow_credits()` 的帧数配额如何随聚合写入扣减

未确认前不应改动 `active_tx: Option<ActiveTx>` 的结构。

### 阶段 4（RX）：消掉中间拷贝

依据第 4.3 节性质 A：`ParsedFrame::Data` 在同一次调用内被完全消费，其 `to_vec()`（`rx.rs:233`）是纯中间拷贝。

**改法**：把 `ParsedFrame` 沿数据面 / 控制面拆开 —— 数据帧借用 `&[u8]`，控制面保持拥有 `Vec<u8>`。或让 `parse_fifo` 接受 visitor 回调，在同一次调用内处理数据帧。

**同时合并 ③④**：`decapsulate_data_frames`（`:578`）非 A-MSDU 路径的 `vec![frame]` 包装可去掉。

**⑤ 不处理**：消掉它需要核心层接受外部提供的写入目标，即让 Driver Core 感知 `DmaBuffer`，破坏不变量 4。

**收益**：每帧省 1 次堆分配 + 1 次拷贝（RX 路径上最大的一次）。

### 优先级

| 阶段 | 收益 | 风险 | 是否碰共享层 | 建议 |
| --- | --- | --- | --- | --- |
| 1 | 中 | 低（取决于 Q1） | 否 | 先做 |
| 4 | 高 | 中 | 否 | 其次 |
| 2 | 中 | 中高 | 否 | 之后 |
| 3 | 高 | 高 | 可能 | 需先确认协议 |

四个阶段相互独立，可分别验证，不需一次做完。

---

## 7. 未确认问题

**Q1（阻塞阶段 1）**：abort 之后 `data.active_tx` 应被清理还是保留？
`finish_cancel()`（`src/device/progress.rs:204-215`）清 mailbox、control、link.peer、internal_tx，但不清 `active_tx`。若 `state` 为 `Starting`，cancel 后变为 `Stopped`；若为 `Ready`，则保持 `Ready` 且 `active_tx` 残留，下一轮 `drive_ready` 会重新 emit `TransmitFlow`。
该行为是否为本意，需依据设计意图判定。

**Q2（阻塞阶段 3）**：firmware 是否接受一次 CMD53 写入多个 host descriptor？需对照 vendor 驱动 `rwnx_tx.c` 与固件行为确认。

**Q3（阻塞阶段 3）**：`flow_credits()` 返回的帧数配额（`src/registers.rs:135-141`）在聚合写入时如何扣减。

**Q4（阶段 2 设计）**：`prepare_offline()` 的准入条件如何精确复刻 `drive_ready` 的优先级？特别是 mailbox 进行中与 RX scan 激活两种状态的判定时机。

---

## 8. 验证方式

每阶段完成后：

```
cargo fmt
cargo xtask clippy --package aic8800
```

并补齐受影响的单元测试。相关既有测试位置：

- TX credit 与退避：`src/device/data_plane.rs:1043`（`transmit_backoff_services_card_irq_without_retrying_credits_early`）、`:1085`、`:1099`（`data_tx_retains_packet_until_credit_reserve_is_available`）
- TX 回收：`src/device/progress.rs:485`（`failure_reclaims_active_and_queued_transmit_tokens_before_terminal_error`）
- RX 预算与事件：`src/device/data_plane.rs:813`、`:848`、`:897`
- RX 解析：`src/rx.rs:257` 起
- IRQ 时序与 CARD_INT 重开：`src/rdif/owner/progress.rs:571` 起（`:577` `transmit_completion_yields_to_card_irq_before_next_sdio_submission`、`:602`、`:610`、`:617`）
- CARD_INT 与 credit 交互：`src/device/data_plane.rs:987`、`:1008`

实体板卡验证使用 SG2002（LicheeRV Nano / AKA-00）目标。
