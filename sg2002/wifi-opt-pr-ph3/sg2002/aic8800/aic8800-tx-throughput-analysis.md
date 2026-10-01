# AIC8800 TX 吞吐瓶颈与异步化方向

分析对象：`drivers/net/aic8800`（SDIO WiFi，SG2002/CV181x）
基线：dev 分支 `2f5347b14`

本文只处理吞吐量问题：**为什么 TX 只有 ~7 Mbps，以及硬件等待时间为什么没有被利用**。
拷贝路径的逐跳清单见同目录 `aic8800-data-plane-optimization.md`；那篇的「优化方案」一节按本文的结论已不成立。

---

## 1. 摘要

1. 25 MHz / 4-bit SDIO 的理论带宽是 12.5 MB/s。实测 TX 7 Mbps = 0.875 MB/s，**总线利用率约 7%**。
2. 因此 TX 是**每帧固定开销受限**，不是带宽受限。93% 的总线时间是空闲的，瓶颈在事务之间的往返，不在传输本身。
3. 三个结构性浪费，按影响排序：
   - **credit 被当成布尔值用** —— 每帧读一次 CMD52 只为了判断"能不能发"，配额值本身从不被使用
   - **`active_tx` 单槽位** —— 上游默认给了 32 深队列，驱动只用 1
   - **等待窗口零工作** —— 等 CMD52 / CMD53 期间没有任何 CPU 准备工作
4. 拷贝次数（每帧 3 次）与上述三者不在同一量级，不是主要设计问题。

---

## 2. 硬件上限与实测差距

DTS 实测值（`os/StarryOS/configs/board/aka-00-sg2002.dtb`，`wifi-sd@4320000` 节点）：

```
compatible     = "cvitek,cv181x-sdio"
bus-width      = 4
max-frequency  = 0x17d7840  = 25 MHz
min-frequency  = 0x61a80    = 400 kHz
src-frequency  = 0x165a0bc0 = 375 MHz
```

25 MHz × 4 bit → 理论 12.5 MB/s。

时钟由平台层一次性设定：`AicFdtProfile::from_info(&info, resources.clock_rate("sdio"))`（`drivers/ax-driver/src/net/aic8800/mod.rs:60`）→ `host_config()`（`drivers/ax-driver/src/cv181x/mod.rs:73-84`）→ `Cv181xConfig { max_frequency_hz, ... }`。卡初始化期间由 `sdmmc-protocol` 协商 `ClockSpeed` 档位，`cv181x-sdhci` 的 `clock_plan()` 把它映射到不超过 `max_frequency_hz` 的 `target_hz`（`drivers/blk/cv181x-sdhci/src/host2.rs:141-158`）。

驱动核心**从不发出时钟变更请求**：`SdioRequestKind::SetClockHz` 在 `src/device/model.rs:143` 定义、在 `src/rdif/owner/operation.rs:137` 有处理分支，但全仓库没有构造点。

---

## 3. 每帧的固定开销

一帧 TX 的完整往返：

```
[协议层提交 DmaBuffer]
  → take_tx_frame：DmaBuffer → Vec<u8>            rdif/owner/output.rs:61-70
  → TxState::enqueue                               tx.rs:26
  → 构造 wire frame                                device/data_plane.rs:443 → protocol.rs:154
  → CMD52 读 flow_control（credit）                 device/data_plane.rs:71-74     ← 事务 1
  → 等 IRQ                                        ← park/wake 1
  → clone wire frame → CMD53 写 FIFO               device/data_plane.rs:388-392   ← 事务 2
  → 等 IRQ                                        ← park/wake 2
  → TransmitComplete 事件                           device/data_plane.rs:405
  → publish_tx_completion → tx_complete 环          rdif/owner/output.rs:160-183
  → 协议层 reclaim token
[下一帧才能进入同一流程]
```

**每帧 2 次总线事务 + 2 次 park/wake + 一次跨线程事件往返。**

量级估算（**非实测**）：25 MHz 下 1.5 KB 的 CMD53 纯传输约 120 µs；加命令、响应、中断周期与驱动路径，单次事务量级 150–250 µs。两次事务加两次唤醒约 0.5–1 ms/帧 → 1000–2000 fps × 1500 B ≈ 12–24 Mbps 上界。实测 7 Mbps 落在该量级内，与"每帧固定开销主导"的假设自洽。

该估算需要用实测替换，见第 6 节。

---

## 4. 三个结构性浪费

### 4.1 credit 被当成布尔值用（影响最大）

`src/device/data_plane.rs:373-394`：

```rust
pub(super) fn consume_transmit_flow(
    &mut self,
    response: SdioResponse,
    now: MonotonicTime,
) -> Result<(), AicError> {
    let credits = self.registers().flow_credits(expect_byte(response)?);
    let active = self.data.active_tx.as_mut().ok_or(AicError::CompletionMismatch)?;
    if credits <= DATA_TX_RESERVED_CREDITS {          // 只用来判断"发不发"
        active.retry_at = Some(now.after(IO_RETRY));
        return Ok(());
    }
    let frame = active.wire_frame.clone();            // 仍然只发一帧
    self.io.next = Some((
        IoPurpose::TransmitData,
        write_fifo(self.data_function(), self.registers().write_fifo, frame),
    ));
    Ok(())
}
```

`flow_credits` 返回的是**帧数配额**，不是布尔（`src/registers.rs:135-141`；解析验证见 `:239-246`，D3 卡上 `v3_flow_credits(0x85) = 133`）。

配额是 133 还是 3，驱动行为完全一致：**发一帧，然后重新发 CMD52 再读一次。**

credit 读在 `drive_ready` 里每帧执行一次（`src/device/data_plane.rs:62-75`）：只要 `active_tx` 存在且 `retry_at` 为空，就 emit `TransmitFlow`。

**结果：每帧两次总线事务中，有一次纯粹是为了换取一个布尔值。**

相关常量：`DATA_TX_RESERVED_CREDITS = 2`（`src/device/data_plane.rs:17`）、`IO_RETRY = 1ms`（`:14`）。

### 4.2 `active_tx` 单槽位 vs 上游 32 深队列

```rust
// src/rdif/device/endpoints/device.rs:23-24
const DEFAULT_QUEUE_SIZE: usize = 32;      // SPSC 环可容纳的 DMA buffer 数
const DEFAULT_FRAME_SIZE: usize = 2048;

// src/device/data_plane.rs:420-423
fn prepare_next_transmit(&mut self) {
    if self.data.active_tx.is_some() { return; }   // 只有一帧在飞
```

`QueueConfig { ring_size: 32, buf_size: 2048, ... }`（`src/rdif/device/endpoints/device.rs:134-139`）由 `queue_parts()` 分配为四组 SPSC 环（`src/rdif/device/queues.rs:31-59`）。

上游备了 32 的流水深度，驱动只用 1。协议层即使塞满 32 帧，驱动也是一帧一发、每帧重读 credit、每帧等两个 IRQ 往返。

### 4.3 等待窗口零工作

等 CMD52 与等 CMD53 期间：

- `advance()` 在 `io.pending.is_some()` 处短路（`src/device/progress.rs:40-42`），`drive_ready()` 不被调用
- 即使被调用，`prepare_next_transmit()` 的第一行也被 `active_tx.is_some()` 挡住

三处等待窗口（等 CMD52、等 CMD53、credit 退避 1 ms）中没有任何 TX 准备工作在执行。

注：`io.pending` 短路本身是正确的（SDIO 总线同时只能有一个事务在飞）。问题在于它**同时挡住了纯 CPU 工作**。

---

## 5. 优化方向

### 方向 A：credit 批量发送

读一次 credit，按配额连续发 N 帧，帧间只插 CMD53、不插 CMD52。

- 总线事务数减半（消除每帧的 CMD52）
- 每帧省掉一次 park/wake
- 需要：`active_tx` 由单槽改为队列；确认固件接受背靠背 CMD53

预估 TX 吞吐 ~2×（**未验证**，取决于 4.1 中 credit 的实际取值）。

### 方向 B：预构造 wire frame 队列

`active_tx` 扩成小队列（4–8 帧），在等待窗口里预构造后续帧的 wire frame。

- 不改变 credit 读取频率，也不改变每帧一次 CMD53 的形态
- 消除「帧 N 完成 → 帧 N+1 才开始构造」的关键路径依赖
- 需要在 `advance()` 的 `io.pending` 短路之前引入纯 CPU 准备步骤，且必须复刻 `drive_ready` 的优先级准入（mailbox > 优先事件 > RX scan > TX）

预估值级 1.2–1.5×（**未验证**）。

### 方向 C：SDIO 时钟

当前 `max-frequency` 锁在 25 MHz（DTS）。若 SG2002 的 SDIO1 控制器硬件支持更高频率，提升该值是最直接的收益。

风险与前提：需要确认硬件能力、信号完整性与 AIC 固件的容忍度；属于平台/DTS 层决策，不是驱动逻辑改动。

### 方向 D：token 回收解耦

`ring_size = 32` 说明 buffer 池足够深，理论上协议层可以在帧 N 未完成时提交帧 N+1。需要确认当前是否因 `publish_tx_completion`（`src/rdif/owner/output.rs:160-183`）的时序而被迫串行。

---

## 6. 待测量与待确认

**必须先测量，再决定实现方向。** 以下两项都不需要改逻辑：

**M1：credit 的实际取值。**
在 `consume_transmit_flow`（`src/device/data_plane.rs:378`）打印 `credits`。

- 若稳定为几十上百 → 方向 A 的收益立即可见
- 若始终为 3–5 → 限流在固件侧，方向 A 收益有限

**M2：SDIO 实际运行频率。**
读 SDHCI 的 clock control 寄存器，或在初始化路径打印编程后的 `target_hz`。若实际低于 25 MHz，方向 C 优先于 A/B。

**M3：每帧的事务耗时分解。**
测量单次 CMD52 与单次 CMD53 的端到端耗时（含 IRQ 往返），替换第 3 节的量级估算。

**Q1（阻塞方向 A）**：firmware 是否接受背靠背 CMD53？需对照 vendor 驱动 `aicwf_sdio.c` 与固件行为。

**Q2（阻塞方向 A）**：`flow_credits` 的扣减语义 —— 一次 CMD53 扣 1，还是按帧或其他单位？

---

## 7. 与 TX 无关但同源的观察

RX 侧存在同构的「长度后置」问题：`ReceiveLength::Blocks(n)` 要等 CMD52 返回后才知道读多少（`src/device/data_plane.rs:150-158`），因此 CMD53 的 DMA 目标无法提前绑定。DC 走 byte mode 时还要先读 `byte_mode_length` 寄存器，多一次往返（`:160-168`）。

底层 `sdmmc-protocol` 已具备拥有型 DMA 能力（`drivers/blk/sdmmc-protocol/src/sdio/io/transfer.rs:26` 的 `SdioDmaTransferRequest<H>` 接收 `PreparedDma`），但 AIC adapter 使用的是只有长度的 `SdioRequestKind::Read`（`src/device/model.rs:129-135`）。该层与 SD 卡路径共用，改动会同时影响块设备。

---

## 8. 相关文档

- `aic8800-data-plane-optimization.md`：TX / RX 逐跳路径、17 个短路条件、拷贝链清单、与 PR #2349 及 SG2002 SDIO 契约共用者的对比。该篇的「优化方案」一节按本文第 4 节的结论已不成立，仅其余部分仍有效。
