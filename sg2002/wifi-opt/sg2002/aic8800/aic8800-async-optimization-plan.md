# AIC8800 数据面异步化（流水线化）方案

分析对象：`drivers/net/aic8800`（SDIO WiFi，SG2002/CV181x）
分析基线：dev `9a7b868ba`（2026-09-23）。工作分支 `sg2002/wifi-opt`：AIC8800 的全部工作（含调试与探针）都在此分支上迭代，2026-09-25 dev 更新到 `714accd8f` 后工作已变基其上；开 PR 时另建去掉探针的整理分支。

本文是执行方案，依据来自同目录三篇分析（结论已逐条对当前 HEAD 复核，见附录 A）：
`throughput-bottleneck-analysis.md`（主依据）、`aic8800-tx-throughput-analysis.md`、
`aic8800-data-plane-optimization.md`。

---

## 1. 目标与判据

### 1.1 目标

把 TX 从「每包一次 CMD52 换一个布尔值 + 停等式」改成「一次读 credit 连发多包 +
下一帧在总线上跑的时候已经备好」，即让 SDIO 总线在还有包要发时保持忙碌。

「异步」在本驱动的既有含义是**不阻塞 CPU**（已达成）；本方案要补的是第二层含义：
**不让硬件闲着**。

### 1.2 判据

| 指标 | 现状（2026-09-16/17 实测） | 厂商 Linux 同链路 | 目标 |
| --- | --- | --- | --- |
| TX TCP 板→PC | 6.6 ~ 7.3 Mbps | 58.5 Mbps | ≥ 40 Mbps（先跨过空口上限，见 §3） |
| TX 总线利用率 | 7 ~ 8 % | 66 % | ≥ 50 % |
| 每包周期 | 1.6 ~ 1.8 ms | 0.20 ms | ≤ 0.25 ms |
| SDIO 事务/包 | 2（CMD52 + CMD53） | ≈ 1/32 CMD53（聚合） | 1（CMD53） |

判据以**板端 iperf3 实测**为准，不以代码形态或单测通过为准。

**用例口径（2026-09-25 修正）**：驱动侧判据取**双向用例的板端 TX 方向**，并以
「每包周期与 SDIO 事务数」作为与代码改动直接对应的指标。纯上行用例同样可用作判据：
周期 P3 在写完成时刻同时采样核心队列与 RDIF 环，实测上行用例里 **88% 的样本「核心队列空、RDIF 环有帧」**、
两层皆空的样本仅 0.03%，即帧早已由协议交给驱动、在环里等待 owner 拉取，限速方在驱动自身
（此前「纯上行受上游供帧限制」的判断基于只采样核心队列，已更正）。

---

## 2. 现状（已对 HEAD 复核）

### 2.1 一帧 TX 的完整往返

```
[协议层提交] → take_tx_frame：DmaBuffer → Vec（读+拷贝）        rdif/owner/output.rs:61
  → 构造 wire frame：vec![0;1536] + 写 hostdesc/头 + 搬 payload   protocol.rs:154-199
  → 读 flow credit（CMD52）                                      data_plane.rs:71-74   事务 1
  → park/wake 1
  → credit ≤ 2 → retry_at = now + 1ms（IO_RETRY）                data_plane.rs:384-387
  → wire_frame.clone() → 写 FIFO（CMD53）                        data_plane.rs:388-392 事务 2
  → park/wake 2
  → TransmitComplete 事件 → 退回 rearm 边界                       rdif/owner/progress.rs:47-65
```

每包 **2 次总线事务 + 2 次 park/wake + 1 次 CMD52 只为一个布尔值**。

### 2.2 三个已核实的结构性浪费

1. **credit 被当成布尔值**：`flow_credits()` 返回的是固件空闲包缓冲数
   （V3 读全字节，最多 128；`registers.rs:135-148`），但 `consume_transmit_flow`
   只用它判断「发不发」，配额值本身从不使用（`data_plane.rs:384`）。
2. **`active_tx` 单槽位**：`prepare_next_transmit` 第一行 `if active_tx.is_some() { return }`
   （`data_plane.rs:420-423`），上一包写完之前下一包的 wire frame 根本不开始构造。
   上游给了 32 深 rdif 环 + 128 深核心队列，驱动只用 1。
3. **等待窗口零准备**：等 CMD52 / CMD53 期间 `advance()` 在 `io.pending` 处短路
   （`progress.rs:40-42`），`drive_ready()` 不被调用，纯 CPU 工作也一起被挡住。

### 2.3 每包的固定开销清单（代码可证）

| 项 | 每包次数 | 位置 |
| --- | --- | --- |
| 堆分配 + 拷贝（DmaBuffer → Vec） | 1 | `rdif/owner/output.rs:65`（`to_vec()`） |
| `vec![0;1536]` 分配 + 清零 + 两次拷贝 | 1 | `protocol.rs:174-198` |
| `wire_frame.clone()` | 1 | `data_plane.rs:388` |
| `CpuDmaBuffer::new_zero` 分配 + 清零 + 拷贝 | 1 | `rdif/owner/operation.rs:114-121` |
| ADMA2 表重建 + 地址重写 | 1 | `drivers/blk/sdhci-host/src/dma/request.rs:349-364` |
| `log_status` 无条件诊断读（9 次 MMIO） | 2 | `drivers/blk/sdhci-host/src/command.rs:338-352,442` |

其中两次「全缓冲清零」是纯浪费（随后整块被覆写）。

---

## 3. 天花板（重要，决定目标值）

25 MHz / 4-bit 的 SDIO 原始上限 ≈ 11.72 MB/s ≈ 85 Mbps；1500 B 帧的数据相 131 µs，
加命令/响应每包约 140 µs。**总线本身足以支撑厂商的 58.5 Mbps**（利用率 66%）。

但**空口速率才是终点**，而它当前没有被确定：

- 驱动的 `me_config_payload`（`lmac.rs:289-306`）只写 HT capability：`payload[3] = 0xff`
  即 MCS 0–7、单空间流；VHT/HE capability 区间（16..100）**全为零**，`payload[103] = 1`
  仅置 HT 标志。
- 厂商 Linux 在同一链路实测 TCP 58.5 Mbps、UDP 73.8 Mbps。HT-MCS7/20 MHz SGI 的
  PHY 上限是 72.2 Mbps，UDP 73.8 已超过它 —— **厂商用了更高的 PHY 速率
  （HE-MCS9 或 40 MHz），本驱动目前不是**。

因此存在两个数量级不同的上限：

| 情形 | TX TCP 可达上限（估） | 本方案能否达到 |
| --- | --- | --- |
| 空口维持 HT-MCS7 | 约 40 Mbps | **能**（流水线化即可） |
| 空口提到 HE-MCS9 | 约 58 Mbps | 不能，需同时修 HE 能力块 |

**结论：流水线化是第一步且必然要做（7 → 约 40 Mbps），但到达厂商水平还需要空口速率侧的工作。**
空口速率本身列为待测项（§4 M4），不在本方案的改动范围内，作为并行议题记录。

SDIO 时钟（25 → 50 MHz）**不作为本方案项**：厂商在同一 25 MHz DTB 下跑到 58.5 Mbps，
说明它不是当前瓶颈；且 `drivers/blk/cv181x-sdhci/src/clock.rs:36-44` 有明确注释说明
25 MHz 以上大块 CMD53 固件写入不可靠。

---

## 4. 阶段 M：先测量（必做，一次上板）

**动机**：实测每包 1.6–1.8 ms，而已知项（1 ms 回退 + 140 µs 总线 + 数百 µs 准备）
只能解释约 1.1–1.2 ms，**还有约 0.5–0.8 ms 没有归属**。在归属明确之前决定实现顺序是猜测。

测量探针做成**临时改动**（不提交，测完即回退），读数走串口日志（限速打印，
每 500–1000 包一行），与既有板测流程一致。

### 4.1 待测量项

| 编号 | 问题 | 探针位置 | 判读 |
| --- | --- | --- | --- |
| **M1** | credit 实际取值分布（min/mean/max）与 `≤2` 触发次数 | `data_plane.rs:378` 处累加 | 常态几十上百 → 批量化收益立刻可见；常态 3–5 → 瓶颈在固件，改走「短重试 + 立即续发」 |
| **M2** | 每包时间分解：enqueue→CMD53 发出、CMD53 发出→完成、完成→下一包 enqueue | `data_plane.rs` 各处已有 `now`，直接累加差值 | 定位那 0.5–0.8 ms |
| **M3** | 上游是否饱和：每次取帧时核心 TX 队列剩余长度 | `tx.rs:33` 处采样 | 恒为 0 → 瓶颈在 TCP 侧，本方案收益被上游限制 |
| **M4** | 空口实际速率档位 | 需外部对照：同一板卡启动厂商 Linux 镜像读 `iw dev wlan0 link` | 判断 §3 的哪一种上限成立 |
| **M5** | credit 回退时的实际等待时长分布 | `data_plane.rs:385` 记 deadline，完成时记实际差 | 区分「1 ms 就是 1 ms」与「定时器/唤醒把它拉长」 |
| **M6** | SDIO 单事务耗时（CMD52 / CMD53）与相邻事务间隔 | `rdif/owner/operation.rs` 提交处 + `consume_*` 完成处 | 直接给出总线忙/闲比例，替代 §2.3 的估算 |

### 4.2 增益预期（不测也成立的部分）

即使没有 M 阶段结论，以下两条不依赖测量结果、可先做（收益上界受 M1 约束）：

- 每包两次清零 → 一次（纯浪费，见 §2.3）；
- `log_status` 的 9 次无条件 MMIO 读按日志级别约束（`drivers/blk/sdhci-host`，与 SD 卡共用但与 AIC 无关的行为等价）。

---

## 5. 阶段 1：credit 路径（预期单点收益最大）

对应 `throughput-bottleneck-analysis.md` §14.5 的 P0 两项。

### 5.1 credit 本地记账

**改法**：`DataPlaneState` 增加 `tx_credits: Option<u8>`（`None` = 未知，必须重读）。

- `consume_transmit_flow` 读到 credit 后记入，并判断 `credits <= DATA_TX_RESERVED_CREDITS`；
- `drive_ready` 的 TX 分支：`tx_credits` 有值且 > 保留值时**跳过 `TransmitFlow` 直接发数据**；
- 每成功写完一包（`consume_transmit_data`）本地减 1，减到阈值置回 `None`；
- 失效条件：mailbox 命令写入（D80 命令与数据共用同一信用池，
  `profile.rs` 的 `MailboxFlowPolicy::CreditGated`）、cancel/reset、启动/连接状态变化、
  任何错误路径。

**正确性依据**：`#2305` 收紧的语义是「固件空闲包缓冲不足以容纳本包时不得写」。
本改动是精确记账而非近似——寄存器报的就是空闲缓冲数，写一包消耗一个；
厂商驱动用的是同一套（`aicwf_sdio_flow_ctrl` + 本地递减，见分析文档 §13.2）。
本地计数只会比「每包重读」更保守，不会更激进。

**保留的既有语义**：完成一包后仍退回 rearm 边界（`progress.rs:47-65`，
`#2299` 的 RX 防饿死设计）不动。

**M1 结论**（`probe-round1-20260924.md` §4.1）：credit 常态 75–119，最大 132、最小 2，
远高于门限，本项成立；不存在「常态 ≤ 3」的退化分支。

### 5.2 缩短 credit 重试粒度

`IO_RETRY` 1 ms → 100~200 µs（`data_plane.rs:14`）。

- 依据：厂商同位置从 200 µs 起递进（分析文档 §7、§13.2），1 ms 不是硬件要求；
  固件排空一包的时间量级（空口 1500 B ≈ 150–200 µs）与 200 µs 同量级。
- 风险：重试本身要付一次 CMD52 往返。若 M1 显示 credit 长期为 0，
  更短的重试会变成「用总线换等待」，收益可能为负 —— **由 M1/M5 决定是否采用**。
- 钉死该行为的单测 `transmit_backoff_services_card_irq_without_retrying_credits_early`
  （`data_plane.rs:1043`）需同步更新。
- **M1/M5 结论**（`probe-round1-20260924.md` §4.1、§4.3）：纯上行回退触发率约 8.7%，
  每次实测 1.36–1.40 ms，比设定的 1 ms 多约 0.33 ms 的到期唤醒开销；credit 不是长期为 0。
  采用区间上界 **200 µs**。

### 5.3 实现状态

阶段 1 已按本节实现（`perf/aic8800-tx-credit-accounting`，
见 `aic8800-optimization-tracker.md` 周期 1）：

- `DataPlaneState::tx_credits` 记录读数，`consume_transmit_data` 每完成一包扣 1，
  降到 `DATA_TX_RESERVED_CREDITS` 丢弃，`drive_ready` 命中缓存即跳过 `TransmitFlow`；
- 失效条件：`MailboxFlow` / `MailboxWrite`（V3 命令与数据共用数据 FIFO）、`Startup` / `Shutdown`、
  `finish_cancel()`；
- `IO_RETRY` = 200 µs。

单测补充两条：一次读取连发多包（缓存逐包递减）、命令转发清空缓存；
`docs/design/unified-sdio-aic8800.md` 的 credit 与退避描述已在同一提交内同步。

板测结论见 `aic8800-optimization-tracker.md` 周期 1：CMD52/包 1.0 → 0.01、回退均值 1.4 ms → 0.53 ms、
上行 TX 包速率 +17%、同口径上行 TCP 6.09 → 9.09 Mbps，无重传；
CMD53 边际往返反而上升约 200 µs，作为阶段 2 的定形依据。

---

## 6. 阶段 2：流水线（把准备移出关键路径）

对应分析文档的 P2/P3 与 `aic8800-data-plane-optimization.md` 阶段 2。

**实测依据（2026-09-25 重测）**：周期 P3 把「完成 → 下一笔发出」的 gap 按**窗口内是否有 RX 事务**分桶后，
发现它是双峰分布：无 RX 事务时 74~81 µs（占 72~78% 的包），有 RX 扫描时 2.1~2.4 ms（占 22~28%）。
周期 P2 记录的 540 µs 是这两者的加权平均，其「由 owner 往返构成」的归因只对空闲那一半成立。
因此**本阶段能消掉的空闲开销上限约 6%**（见跟踪文档周期 P3 结论 1），
不作为下一步主线；§6.1~§6.3 的机制保留为后续小项，等聚合与每笔事务成本两项做完再评估。

### 6.1 机制：允许核心在事务挂起时做纯 CPU 准备

现状 `advance()` 在 `io.pending.is_some()` 处短路（`progress.rs:40-42`），
`drive_ready()` 不被调用，因此等待窗口内没有任何 TX 准备。

**改法**（两处，都在既有契约内）：

1. 核心：在 `advance()` 的 `io.pending` 短路**之前**插入纯 CPU 准备步骤
   （不碰 `io`、不碰 `irq_latch`、不返回动作、幂等）。其准入必须复刻
   `drive_ready` 的优先级（mailbox > 优先事件 > RX scan > TX），否则破坏 RX 优先语义。
2. 适配层：`AicOwner::consume_action` 在 `SubmitSdio` 返回 `Pending` 时，
   不要立即 `return Wait`，而是先用 `AicInput::tick(now)` 再推进核心一次
   （核心此时被 `io.pending` 挡住不会提交新事务，只会先做纯 CPU 准备），
   然后再返回等待。

### 6.2 双帧暂存：完成即续发

`active_tx` 之外增加一个深 1 的 `staged_tx`：

- 一轮里的顺序变成：收割完成 → `staged` 转 `active` 并立刻发 CMD53 →（此时总线忙）
  准备下一帧进 `staged`；
- 消除「帧 N 完成 → 帧 N+1 才开始构造」的关键路径依赖（分析文档 §14.4 的 L）。

### 6.3 适配层的固定暂存

`rdif/owner/operation.rs:114-121` 每包 `new_zero` 分配 + 清零 + 拷贝，
且地址每次都变，迫使 `sdhci-host` 每包重建 ADMA2 表。改为**复用的固定暂存缓冲**
（单事务模型保证上一笔已完成），去掉分配、清零与地址重算。

### 6.4 不改的

`ActiveTx` 的单槽语义、`CardIrqWait`、`rearm_and_check` 的 completion-before-card 顺序、
`rdif_eth` 的 `NetDeviceParts` / `NetPollGroupParts` 契约，
以及「Driver Core 不依赖 RDIF / `DmaBuffer`」的分层（`docs/design/unified-sdio-aic8800.md`）。

---

## 7. 阶段 3：关键路径清理

1. **去掉两次全缓冲清零**：`protocol.rs:174` 的 `vec![0; final_len]` 改为按需构造
   （头/描述符/payload 三段写满，尾部 padding 仍需清零时只清尾部）；
   `CpuDmaBuffer::new_zero` 改为 `new` 后整块覆写。
2. **`log_status` 的诊断读按日志级别约束**：
   `drivers/blk/sdhci-host/src/command.rs:338-352` 先读 9 个寄存器再 `log::debug!`，
   读操作不受级别保护。改为先判级别再读。该项每包约 18 次无用 MMIO。
   （与 SD 卡共用，但只是把「无条件读」改成「按级别读」，行为等价。）
3. **`take_tx_frame` 的 `to_vec()`**：`rdif/owner/output.rs:65` 每包一次分配 + 拷贝。
   评估是否可让 `DmaBuffer` 直接进入核心（会破坏分层，倾向不做）或复用缓冲。

---

## 8. 阶段 4：聚合（2026-09-25 优先级上调）

周期 P3 的实测把这一项从「二阶项」推到了主线上：写方向的成本随帧长剧增
（512 B 约 148 µs，1456~1532 B 约 679~700 µs），而读方向已经是「一次事务带多帧」的形态
（下行用例每次读平均 10 KB，成绩 26.7 Mbps）。也就是说 TX 目前是**一包一笔 CMD53**，
每字节成本是读方向的 3~6 倍；厂商同硬件跑出 58.5 Mbps 靠的正是 32 包/次 CMD53。

周期 P4 的读数（写成本由帧长决定）与周期 P5 的最小实现已把立项形式定下来：
一笔 CMD53 携带 K 个完整线上帧，K 由适配层给出并被缓存 credit 二次约束（`aggregate_limit()`）；
周期 P6 修复了该实现首轮审查的结论。

**硬约束（周期 P6 审查确证）**：固件把一笔写当作帧流遍历，步长为 `4 + align4(声明长度)`，
读到 `packet_len == 0` 终止。厂商驱动逐帧只做 4 字节对齐、仅在整笔末尾补齐到 512
（`aicwf_sdio.c:2214-2247`、`aicwf_sdio_aggr_send`），本驱动的接收解析器（`rx.rs`）同构。
聚合写必须遵守同一布局：逐帧补齐会让遍历在第 1 帧后终止，**第 2..K 帧被静默丢弃**，
而 CMD53 正常完成、token 与 credit 照常扣减、日志无任何错误。

仍未定：credit 的单位（厂商命令路径按字节检查，本驱动按包扣减）与冲刷策略
（延迟与 RX 优先 `#2299` 的平衡需要论证）；`docs/design/unified-sdio-aic8800.md` 的保证需同步。

---

## 9. 明确不做

1. **不引入线程、不自旋、不睡在驱动里**。厂商的 200 µs 级 `udelay` 退避依赖内核线程上下文，
   与 `docs/design/unified-sdio-aic8800.md` 的契约（核心不创建线程、不持 OS 锁、不调用
   sleep/yield）冲突。要的是它的**策略**（本地记账 + 短周期恢复），不是它的实现。
2. **不改与 SD 卡共用的 `drivers/blk` 契约**，除 §7.2 的等价日志级别约束。
3. **不改 SDIO 时钟与 DTB**（理由见 §3）。
4. **不改 `active_tx` 的单槽语义去换取吞吐**，除非聚合项（§8）按同一份数据论证过
   RX 优先（`#2299`）仍成立。
5. **不改 `rdif_eth` 对外接口与 IRQ 时序。**

---

## 10. 验证与提交

### 10.1 每阶段的检查

```
cargo fmt
cargo xtask clippy --package aic8800
```

受影响单测需同步更新，至少覆盖：credit 记账的失效条件、`staged_tx` 与取消/失败路径的交互、
完成事件与 RX scan 的优先级顺序。既有相关测试位置见分析文档 §8 与 §14.5。

### 10.2 板测

每阶段一次：同一块板、同一条链路、同一台 PC 对端，跑 iperf3 三方向
（TCP tx / TCP rx / UDP tx），与 §1.2 的基线对照。
TX 提升是各阶段的验收指标；RX 若同步劣化则回退。
（UDP 用例因上层会卡死板子，暂以 TCP tx / rx / bidir 代替，见跟踪文档。）
其中**驱动侧验收以双向用例的板端 TX 方向为主**，理由见 §1.2 的用例口径；
纯上行用例同样反映驱动自身的每包周期，可作为辅助判据（实测在 3.56 ~ 9.50 Mbps 之间波动，
波动来自用例期间的上层行为，判读时看同一轮内的相对变化）。

### 10.2.1 收益拆解（A/B 对照）

阶段内多项改动叠加时，另建一版只回退**单项**的对照镜像测同一套用例，
把该阶段的收益拆开记，并给出与主线 dev 相比的净收益。对照版本走临时分支，不进正式提交。
首轮对照项：`IO_RETRY` 回到 1 ms（只改一个常量），用于分离「credit 记账」与「回退粒度」。

### 10.3 提交

每个阶段一个 PR，`type(scope): content` 英文标题、中文正文，正文含背景、改动点、
本地验证与板测结果。设计文档 `docs/design/unified-sdio-aic8800.md` 中
「运行期的数据包退避由 `ActiveTx::retry_at` 持有」「不会提前反复读取 credit」
等描述在阶段 1、2 后需同步更新。

---

## 11. 风险

| # | 风险 | 影响 | 缓解 |
| --- | --- | --- | --- |
| R1 | credit 常态极小，阶段 1 收益落空 | 单点最大预期失效 | M1 先测；退化为 5.2 + 阶段 2 |
| R2 | 每包空转属调度/唤醒而非驱动事务本身 | 阶段 1、2 收益有限 | 已由 P2 实测：驱动受限用例里 gap ≈540 µs 由 owner 往返构成；纯上行还需 P3 区分「栈没产帧」与「帧在上一层等 owner 拉」。必要时转入 `net/ax-net` 的供给/唤醒侧 |
| R3 | 本地记账与固件实际扣减不同步 | 超发被固件丢包（静默丢包） | 阈值保守 + mailbox 写入即失效 + 板测看丢包/重传 |
| R4 | 纯 CPU 准备破坏 RX 优先语义 | RX 饥饿（`#2299` 回归） | 准入条件复刻 `drive_ready` 优先级 + 定向单测 |
| R5 | 上游（TCP）本就供不上帧 | 全方案收益被上游封顶 | M3 先测 |
| R6 | 空口上限低于厂商（§3） | 达不到 58.5 Mbps 判据 | M4 先测；作为并行议题，不混入本方案 |

---

## 12. 后续方向（超出本方案，未立项）

本方案在现有驱动设计边界内调整流程与补充操作。若阶段 2 之后实测显示
「每包准备时间」仍占周期主导，即在边界内已到顶，届时考虑突破边界本身，
以更直接地服务硬件异步（数据相准备、多笔在途、预置暂存等）：

- 单 owner、核心不建线程不阻塞 → 提交与收割分离；
- `active_tx` 单槽、完成即回退 rearm 边界 → 连续提交多笔 CMD53；
- 核心不持有 DMA 缓冲（适配层每包准备）→ 预置多帧暂存 / 描述符链。

代价是完成匹配、credit 记账、取消与 RX 优先语义都要重新论证，
`docs/design/unified-sdio-aic8800.md` 的保证需重述；因此以阶段 2 的实测为立项门槛，
不在本方案内预做。

---

## 附录 A：www 文档核对结论

对当前 HEAD `9a7b868ba` 逐条复核。分析基线 `18ca1d2d4`（09-18）与 `2f5347b14`（09-20）
到 HEAD 之间，`drivers/net/aic8800` 与 `drivers/blk` **没有任何改动**，`net/ax-net` 只有
版本号与 CHANGELOG 变化，因此三篇分析文档的代码结论与行号仍然有效：

| 文档 | 结论 |
| --- | --- |
| `throughput-bottleneck-analysis.md` / `-pub.md` | **现行，本方案主依据**。§12 实测、§13 厂商对照、§14.5 优先级均已复核成立 |
| `aic8800-tx-throughput-analysis.md` | **现行**。方向 A/B/C/D 与 M1–M3 是本方案 §4–§8 的直接来源；「取代前一篇优化优先级」的声明正确 |
| `aic8800-data-plane-optimization.md` | **部分现行**：逐跳路径、17 个短路条件、分层对比仍准确；§6 优化方案与优先级已被前两篇取代（该文档自身已标注） |
| `aic8800-driver-principles.md` | 现行（分层原理） |
| `licheervnano-bus-topology.md`、`sg2002-ap-investigation.md`、`draft.md` | 与数据面优化无关，保持现状 |
| `sg2002/iperf3-tests/` | **基线数据来源**，§1.2 的数字出自此 |
| `before-refactor/*`、`before-refactor/teaching/*` | **过时**：全部基于重构前实现（5 线程 / PollSet / kicker / PIO / 单把 SDIO 锁 / `components/aic8800` 路径），这些机制在当前代码中已不存在。可借鉴项见下方「仍有价值」 |
| `sg2002-wifi-irq/newdev/*` | **已合入 dev**：`#2276` / `4713bfc98`（2026-09-11），其 d80 修复均在当前基线。`www/README.md:27` 的「不在当前 dev」表述与此不符，需更正 |
| `sg2002-wifi-irq/archive/*` | **过时**（代码路径已删除或改名）；`9-4-after-wifi-refactor/aic8800-async-pipeline-design-20260822.md` 的「信用事件必须有生产者」分析仍有概念价值 |

### 仍有借鉴意义的过时结论

1. **空口速率**：`before-refactor/sg2002-wifi-performance-analysis.md` 把 HE 数据通路列为
   最大单一变量，该结论在当前 HEAD 仍然成立（`lmac.rs:289-306` 从不写 HE 能力块），
   是 §3 天花板问题的最早来源。
2. **eBPF / tracepoint 监测**：`before-refactor/wifi-analysis.md` §3 的埋点方案至今未实现；
   `components/ax-tracepoint` 现已在树内，是 §4 测量在后续迭代中的正规化路径（本轮先用日志）。
3. **厂商信用语义**：`sg2002-wifi-irq/archive/9-4-after-wifi-refactor/aic8800-async-pipeline-design-20260822.md`
   §3.7 的厂商对照（命令与数据共用信用池、信用值即聚合预算、D80 信用读 Q1）已复核成立，
   直接支撑 §5.1 的失效条件设计。
4. **固件异常恢复**：多篇旧文档提到 `fail()` 后设备停在 `Failed` → `Idle`，只能重启。
   当前仍然如此（`device/progress.rs:217`），与本方案独立，可在数据面稳定后单独立项。

---

## 附录 B：关键代码位置（相对仓库根）

| 主题 | 位置 |
| --- | --- |
| 核心推进（单动作契约） | `drivers/net/aic8800/src/device/progress.rs:21-58` |
| ready 态优先级（mailbox > 事件 > RX scan > TX） | `drivers/net/aic8800/src/device/data_plane.rs:22-77` |
| credit 检查与 CMD53 写 | `drivers/net/aic8800/src/device/data_plane.rs:373-402` |
| 单包准备（当前单槽位） | `drivers/net/aic8800/src/device/data_plane.rs:420-463` |
| wire frame 构造 | `drivers/net/aic8800/src/tx.rs:33-45`、`protocol.rs:154-199` |
| credit 寄存器语义与取值 | `drivers/net/aic8800/src/registers.rs:135-148`、`profile.rs:100-125` |
| owner 步进循环与 rearm 边界 | `drivers/net/aic8800/src/rdif/owner/progress.rs:41-65,236-310` |
| CMD53 提交与暂存缓冲 | `drivers/net/aic8800/src/rdif/owner/operation.rs:106-136` |
| ADMA2 重建 | `drivers/blk/sdhci-host/src/dma/request.rs:349-364` |
| 队列线程轮询与 finish_idle | `net/ax-net/src/queue_runtime/executor/mod.rs:879-941` |
| 架构契约（不变量） | `docs/design/unified-sdio-aic8800.md` |
