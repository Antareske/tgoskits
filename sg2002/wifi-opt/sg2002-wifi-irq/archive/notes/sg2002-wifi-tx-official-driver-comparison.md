# SG2002 WiFi TX 上行：Starry 与官方 Linux 驱动对照分析

## 背景

- **硬件**: LicheeRV Nano (SG2002, C906 单核 RISC-V)，WiFi 模组 aic8800 (SDIO 4-bit, 25MHz)
- **Starry 驱动**: `components/aic8800/` + `components/sdhci-cv1800/`，PIO 模式
- **官方驱动**: `www/LicheeRV-Nano-Build/osdrv/extdrv/wireless/aic8800/`，荔枝派官方 BSP
- **测试结果来源**: `sg2002-wifi-tx-rx-iperf3-analysis.md`
- **比较目的**: 审查官方 Linux 驱动的 TX 路径实现，对 Starry 当前 ~0.85 Mbps 的上行瓶颈是否有借鉴价值

## 架构差异总览

| 维度 | 官方 Linux 驱动 | Starry 当前 |
|------|----------------|-------------|
| TX 调度模型 | 专用内核线程 `aicwf_bustx_thread`，可配 `SCHED_FIFO` RT 优先级 (`aicwf_sdio.c:2506`) | 异步 poll task (`wifi-tx`)，依赖 executor 调度 (`tx.rs:45`) |
| 唤醒机制 | `wait_for_completion_interruptible()` — 数据入队时精确 `complete(&bus_if->bustx_trgg)` 唤醒 (`aicwf_sdio.c:2010`) | PollSet 边沿唤醒 (`wake_pollset.wake()`) + 10ms kicker 兜底（仅 DC/DW）(`tx.rs:95-113`) |
| SDIO 写方式 | `sdio_writesb()` / `sdio_write_sg()` → 内核 MMC 子系统 → cvitek host 驱动（ADMA） | 自行实现 `pio_write()`：逐 32-bit word 写 `SDHCI_BUFFER`，逐 block 等 `BUF_WR_READY` (`sdhci-cv1800/src/lib.rs:503`) |
| TX 帧聚合 | scatter-gather 聚合，每次最多 32 帧合并为一次 `sdio_write_sg()` (`aicwf_sdio.c:2095-2100`) | 无聚合，每帧独立调用 `write_fifo()` → `cmd53_write_fixed()` (`tx.rs:336`) |
| 流控模型 | firmware buffer 信用计数（`fw_avail_bufcnt`），值域 0~32，精确跟踪可用空间 | 流控寄存器阈值检查（`fc > DATA_FLOW_CTRL_THRESH = 2`），无信用跟踪 (`tx.rs:256`) |
| 流控等待策略 | 渐进退避：`<30` 次 `udelay(200)` → `msleep(2)` → `msleep(10)` (`aicwf_sdio.c:380-385`) | 统一 `yield_now()` × 50 (`tx.rs:256-268`) |
| 网络层流控 | `tx_fc_low_water` / `tx_fc_high_water` 高低水位 → `netif_tx_stop/wake_all_queues` (`aicwf_sdio.c:1957-1966`) | `MAX_TX_QUEUE_LEN = 256` 硬上限 → `QueueFull` 丢帧 (`tx.rs:550-551`) |

## 关键差异逐项分析

### 1. TX 帧聚合 — 最具潜力的架构改进

官方驱动的 `aicwf_sdio_send()` (`aicwf_sdio.c:2065-2149`) 将多个 skb 累积到 scatter-gather list：

```
aicwf_sdio_send()
  ├─ aicwf_sdio_aggr()           // 累积 skb 到 tx_priv->sg_list[]
  └─ aicwf_sdio_aggr_send()      // 触发条件满足时一次性发出
       └─ aicwf_sdio_txscatterpkt()  // sdio_write_sg() → 单次 CMD53
```

触发聚合发送的条件（`aicwf_sdio.c:2138-2140`）：

```c
if ((int)atomic_read(&tx_priv->tx_pktcnt) == 1    // 仅一帧时立即发
    || txnow                                         // 强制立即发（CMD 在等）
    || atomic_read(&tx_priv->aggr_count) >= tx_aggr_counter  // 达到聚合上限（默认 32）
    || (atomic_read(&tx_priv->aggr_count) ==
        (tx_priv->fw_avail_bufcnt - DATA_FLOW_CTRL_THRESH)))  // 填满 firmware 可用信用
```

每 ~32 帧做一次 CMD53 事务（`cmd53_xfer` + `wait_cmd_complete` + `pio_write` + `wait_transfer_complete`）。

Starry 每帧独立走 `write_fifo()` → `cmd53_write_fixed()` → 完整的 CMD53 事务链。一次 128 KB burst（~85 帧 × ~1500B）在 Starry 中产生 85 次 CMD53，官方聚合后仅约 3 次。CMD53 每次有固定的寄存器写入开销和 `wait_data_idle()` 自旋开销。

**借鉴方向**：在 Starry 的 `process_data_tx()` 中累积 `batch_count` 帧到合并 buffer，达到 N 帧或流控上限后一次 `write_fifo()` 发出。`build_data_frame` 需要支持将多帧 payload 串联为一个 SDIO payload（多个 hostdesc + payload 对）。

### 2. 流控等待：渐进退避模式

官方驱动的 `aicwf_sdio_flow_ctrl()` (`aicwf_sdio.c:351-390`)：

| 重试区间 | 等待方式 | 时间 |
|---------|---------|------|
| 1–29 | `udelay(200)` | 200 µs |
| 30–39 | `msleep(2)` | 2 ms |
| 40+ | `msleep(10)` | 10 ms |

设计意图：硬件大概率在微秒级就绪（快速路径），但如果 firmware 确实繁忙则逐步降低 CPU 占用（慢速路径）。

Starry 的 `check_data_flow_control()` (`tx.rs:255-268`) 统一 `yield_now()` × 50，在快速路径上浪费了 executor 调度开销，而在慢速路径上又可能过于密集。

**借鉴方向**：采用分层重试——前 20 次 `spin_loop()`，之后逐步引入 `yield_now()` 或 sleep。

### 3. 信用制流控 vs 简单阈值

官方驱动维护 `fw_avail_bufcnt`，每次 `aicwf_sdio_flow_ctrl()` 从硬件寄存器读取并用 `fc & SDIOWIFI_FLOWCTRL_MASK_REG` 得到实际信用数。发送后将已发帧数从信用扣除。这使得驱动**精确知道还能发多少帧**，用于：
- 聚合批量大小决策（不超过可用信用）
- 提前终止 TX loop（信用不足时不强发，避免固件丢帧）

Starry 的 `DATA_FLOW_CTRL_THRESH = 2` 只做了"有空间 / 无空间"的二元判断，不知道具体还有多少 buffer。这在以下路径产生问题：
- `process_data_tx()` 中的 `check_data_flow_control()` 通过后，`send_single_data_frame` 可能因为流控实际不足而失败
- 聚合实现时（见第 1 点）无法做精确的批量大小决策

**借鉴方向**：将 `check_data_flow_control()` 改为返回 `fc_value`，在 `process_data_tx()` 的 while 循环中递减计数。

### 4. TX 线程优先级

官方驱动可配置 `bustx_thread_prio > 0` 时使用 `SCHED_FIFO` RT 调度 (`aicwf_sdio.c:2530-2537`)。目的是保证 TX 线程不被其他内核线程抢占——在单核系统上，TX 延迟直接影响吞吐。

Starry 的 `wifi-tx` 是标准 async poll task，与其他任务（如 TCP 协议栈、RX 处理）共享 executor 调度时间片。在单核 C906 上，TX 可能在数据就绪时未被及时调度。

**借鉴方向**：在当前 async 框架下，此差异不太可能直接消除。但通过缩短等待粒度（P0 级改进）可以大幅降低任务被阻塞的时间，减少调度延迟的影响。

### 5. SDIO 写路径：PIO 等待粒度（已识别为 P0 瓶颈）

这是当前 Starry 性能退化的直接原因。详细分析见 `sg2002-wifi-tx-rx-iperf3-analysis.md` 第六节。

官方驱动调用 `sdio_writesb()` → Linux MMC 子系统，其内部使用 ADMA 或 PIO（取决于 host 能力）。参考 BSP 中 cvitek sdhci 驱动，PIO 模式下 `BUF_WR_READY` 等待使用自旋（`read_poll_timeout` / 直接轮询 `present_state`），不引入毫秒级睡眠。

Starry 在 `sdhci-cv1800/src/lib.rs:194-196`：

```rust
// 非 XFER 位（BUF_WR_READY / CMD_COMPLETE）：
crate::runtime::delay().delay_ms(PHASE2_STEP_MS); // 10ms 粗粒度睡眠
```

单次 128 KB TX 中，即使仅 10% 的 block 落入 Phase 2（~25/256 blocks），累积延迟即达 250ms。官方驱动的 PIO 路径无此问题。

**借鉴方向（P0）**：非 XFER 位的 Phase 2 改为 `yield_now()` 或 `spin_loop()` 自旋，与 Part 1 忙等方案对齐。

## 借鉴价值排序

| 优先级 | 改进方向 | 预期效果 | 复杂度 | 官方代码锚点 |
|--------|---------|---------|--------|-------------|
| **P0** | BUF_WR_READY / CMD_COMPLETE 的 Phase 2 `delay_ms(10)` → 自旋或 `yield_now()` | 恢复 Part 1 级别的 ~10 Mbps TX 吞吐 | 低（常量替换） | 间接参照 Linux cvitek sdhci PIO 路径的 `read_poll_timeout` |
| **P1** | TX 帧聚合：多帧合并为一次 CMD53 | 大幅减少 CMD53 事务数（85→3/128KB），与 P0 叠加改善流控反压 | 中（需修改 `build_data_frame` 支持多帧串联） | `aicwf_sdio_aggr()` / `aicwf_sdio_aggr_send()` (`aicwf_sdio.c:2152-2287`) |
| **P2** | 信用制流控（维护 `fw_avail_bufcnt` 计数） | TX pacing 精准化，减少流控不足时强发导致的丢帧 | 中 | `aicwf_sdio_flow_ctrl()` (`aicwf_sdio.c:351-390`) |
| **P2** | 流控等待渐进退避（`spin_loop` → `yield` → `sleep`） | CPU 效率优化，快速路径延迟更低 | 低 | `aicwf_sdio_flow_ctrl()` 重试逻辑 (`aicwf_sdio.c:380-385`) |
| **P3** | TX 线程优先级（RT 调度） | 单核场景下减少 TX 调度延迟 | 高（当前 async 框架下不易实现） | `bustx_thread_prio` + `SCHED_FIFO` (`aicwf_sdio.c:2530-2537`) |

## P0 + P1 的配合关系

P0 和 P1 解决不同瓶颈，效果叠加：

- **P0 单独**：消除 `delay_ms(10)` 的硬延迟，TX 每帧 3 blocks 的 CMD53 等待从 max ~60ms 降至 ~150µs。预期恢复至接近 Part 1 的 ~10 Mbps。
- **P1 单独**：减少 CMD53 事务数，但每次 CMD53 内部的 `delay_ms(10)` 延迟从 85 次变为 3 次——总延迟仍然可观。
- **P0 + P1 同时**：CMD53 事务减少（3 次/128KB）+ 每次事务等待极短（微秒级），预期 TX 吞吐超过单独 Part 1 的水平。

P0 建议作为先行修复（改动最小、收益最大）；P1 基于 P0 的结果决定是否需要进一步优化。

## 不适用或无需参考的部分

以下官方驱动的特性当前无需或不应参考：

- **TX CFM（确认）机制**：Starry 已正常工作，不需要修改
- **A-MSDU 聚合**：官方驱动支持 802.11 A-MSDU 聚合（`rwnx_tx.c`），这是 802.11 层的聚合，与 SDIO 传输层的帧聚合不同。在 TX 吞吐达到 ~20 Mbps 前意义不大
- **ADMA 模式**：SG2002 的 SDHCI 支持 ADMA2（`block_path.rs` 中有 `submit_write_adma2`），但当前仅块设备路径使用。WiFi SDIO 切换到 DMA 可进一步消除 PIO 开销，但属于较大的架构改动
- **TCP ACK 过滤**（`CONFIG_FILTER_TCP_ACK`）：在 TCP→WiFi 的上行路径中减少冗余 ACK 帧，当前瓶颈不在 ACK 数量
