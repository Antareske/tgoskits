# AIC8800 异步双缓冲重构方案与交付

本文是 **AIC8800 CPU/SDIO 协作式异步流水线（"双缓冲"）**的正式方案与交付记录。
方案源自本轮会话规划阶段留下的原始记录 `async-sdio-session-original-plan.txt`；本文在保持其
范围与结论的前提下统一了术语、补上源码依据，并把"已实现"与"待实现"分开陈述。
第 5、6、7 节随实现推进更新；接手所需的操作细节见 `handoff-20260929-async-stage.md`。

## 1. 交付范围

本轮针对 AIC8800 驱动层做一次较大范围重构，目标是让 **CPU 侧的构帧、聚合、DMA 填充与
cache publish 落入当前 SDIO 事务的等待窗口**，而不是像现在这样排在事务完成之后。

"双缓冲"在本工作线里的含义是 **一个 host 事务在飞 + 一个 CPU-owned 后继槽**，不是两笔
CMD53 并发在飞。三条独立证据决定了这个边界：

| 证据 | 位置 | 结论 |
| --- | --- | --- |
| 传统 SDHCI 只有一个在途硬件请求 | `drivers/blk/sdhci-host/README.md` | `queue_depth = max_submit_batch = 1` |
| 主机 trait 按单事务建模 | `drivers/blk/sdmmc-host/src/lib.rs` | "A host accepts one transaction at a time" |
| IO 卡以唯一活动请求号拒绝第二笔 | `drivers/blk/sdmmc-protocol/src/sdio/io/mod.rs` | `active_io_request_id`，繁忙返回 `Error::Busy` |

因此本轮**不**扩展为多笔总线事务并发、不新增 owner 线程、不引入 Rust `Future`，也不把硬中断
变成数据处理上下文。第一阶段（§5.1）交付的是这条流水线的 CPU 侧一半——核心在飞窗口内成形
下一笔；owner 侧准备好的 DMA backing 是下一步。

## 2. 问题与目标

### 2.1 当前形态

一笔写从协议帧到总线依次经过：`OwnerOutputs::take_tx_frame()` → `AicInputEvent::TxBatch`
→ `AicDevice::prepare_next_transmit()` 成形 wire 流 → `take_write_bytes()` 复制出 CMD53
字节 → `AicAction::SubmitSdio` → `ActiveOperation::submit()` 分配 `CpuDmaBuffer` 并复制
→ `SdioCard::submit_write_dma()` → SDHCI host。

`AicDevice::io.pending` 只有一个活动请求，`AicOwner::active: Option<ActiveOperation>`
只有一个活动主机操作，`AicDevice::data.active_tx` 既表达"在总线上那一笔"又表达"正在成形的
下一笔"。后果是：**成形与 DMA 准备被安排在完成之后**，它们花掉的每一微秒都是总线空档。

### 2.2 目标模型

```text
CPU:   prepare A -> submit A -> prepare B -------- commit B -> prepare C
SDIO:                   transfer A -------------- transfer B --------
```

已有机制（`continue_transmit_pipeline`、`submit_one_tx` 在飞期间交帧）已经把**取帧**提前到
在飞窗口内，但没有把**成形与 DMA 填充**提前。本轮补的就是这一段。

### 2.3 收益口径

第四阶段实测（`stage-delivery-20260930.md` §3.5）给出的可动空间是：上行墙钟中事务在飞
38%、"哪里都没有帧" 20%、"有帧却在等" 23%（其中 credit 自选等待 16.1%，余约 7% 为
完成→中断→交接→再发的固有延迟）。**搬运方式本身已经不是可动项**（每字节 89 ns，约物理
底速的 96%），差距只在占空比。

由此可以判定本轮的收益上界与不应混入的变量：

- 双缓冲能回收的是**成形 + DMA 准备 + 提交**这段固有延迟，量级是每笔写约 0.5–0.6 ms
  （`attrib2` 探针：`pull + form + bytes` 0.38–0.47 ms，DMA 准备 0.14–0.16 ms）；
- credit 自选等待（16.1%）是**固件缓冲池策略**，双缓冲不改变它；板测必须先用同会话
  `aic,tx-credit-wait-us=0` 对照把它单独归因，不能与双缓冲效果算作同一个变量；
- "哪里都没有帧" 20% 由协议栈的突发式交付决定，不属于本轮。

## 3. 方案

### 3.1 单一 TX 所有权状态机

`drivers/net/aic8800/src/device/owner.rs` 中 `ActiveTx` 目前把三类职责混在一个槽里：
"已成形待提交的批次"、"已提交事务的完成元数据"、"提交尝试的退避状态"。方案把它拆为：

| 对象 | 所有者 | 责任 |
| --- | --- | --- |
| `TxWrite`（由 `ActiveTx` 拆出） | `AicDevice` | wire 流、`stream_len`、`direct`、`bulk`、完成归属（`TxCompletion`）、随行 token |
| 提交尝试状态（`retry_at`、`credit_waits`） | `AicDevice` | 只描述"这笔待提交的写"的退避，不描述写本身 |
| `StagedTx` | `AicOwner` | CPU-owned DMA backing、有效长度、批次身份 |
| `InFlightTx` | `ActiveOperation` | 已交给 host 的请求及其完成元数据 |
| `CompletedTx` | `AicOwner` | quiesce 后回收的 backing 与待发布 token |

不变量：

- 每个 move-only TX token 在任一时刻只有一个权威所有者；
- `wire_frame`/`stream_len`/`direct` 留在协议核心，`AicDevice` 不持有 `DeviceDma`、操作系统
  锁、任务或执行器；
- 保留 wire 格式、聚合上限、普通数据 `hostid=0`、credit 的包单位以及既有 RX/控制协议语义；
- 删除不再需要的可克隆请求与缓存副本。

### 3.2 请求在飞期间准备下一笔

主要接入点是 `rdif/owner/progress.rs::advance_with_cause`、`submit_one_tx` 与
`rdif/owner/operation.rs::ActiveOperation::{submit, advance}`。

每个 owner step 的既有顺序保持不变：已确认 IRQ → 在飞事务推进/完成 → 控制请求 →
RX/mailbox → 取帧 → 成形。变化只在"成形"这一步的前提下：当 `in_flight` 非空时，
**允许有界地形成至多一个后继批次**，但不产生第二个 SDIO action，也不消费 credit。

触发沿用既有固定 CPU owner 通知路径（`WakeReason::Transmit` 与 sticky notification），
**禁止**新增定时轮询或额外执行线程；`net/ax-net/src/queue_runtime/executor/mod.rs` 的
接入仅限确实需要的调度语义，不触碰或削弱固定亲和性与丢失唤醒闭环。

### 3.3 DMA 双槽与活动前缀契约

当前 `memory/dma-api/src/owned.rs::CpuDmaBuffer::prepare_for_device()` 把**整个 backing**
当作活动长度，`PreparedDma::len()` 直接返回 backing 长度；`sdmmc-protocol` 的
`submit_dma()` 用它决定 CMD53 的 block/byte 形状，`sdhci-host` 的
`build_prepared_adma2_data_request()` 用它校验数据形状
（`validate_adma2_data_shape(block_size, block_count, len)`）并编程 ADMA2 描述符长度。
因此固定容量 backing 不能直接提交为较短的聚合，否则会把尾部无效数据一并交给设备。

方案是在 `dma-api` 增加经过审查的 **活动前缀**状态转换：

- `CpuDmaBuffer::prepare_prefix(len)`：检查 `0 < len <= capacity`，只同步 `0..len` 范围；
- `PreparedDma`/`InFlightDma`/`CompletedDma` 全链路携带活动长度，`len()` 返回活动前缀，
  另提供 `capacity()` 返回完整 backing；
- `segment()`/`segments()` 按活动长度表达；
- `into_cpu_buffer()` 与完成后恢复完整可写 backing；
- 提交拒绝原样归还 `PreparedDma`（既有 `SdioDmaSubmitError::into_parts()` 已满足）。

若某个 host 层无法无损表达活动长度或 cache 范围，该 host 必须**明确拒绝**该能力
（返回 `Error::UnsupportedCommand`），而不是超长传输或缩窄成不健全的 `unsafe` 约定。

临时实验方案（按实际 CMD53 block 长度分配精确大小的 backing）只作为退路：它不改公共接口，
但把分配次数与容量上界留在调用方，不应演变成长期的尺寸池碎片化设计。

### 3.4 提交门控与优先级

`Staged -> Submitted` 是唯一的裁决点：此刻才重新读取权威 credit、检查命令保留量、当前 TX
聚合上限与取消/停止状态，之后才消费 credit 并转移 DMA 所有权。**信用快照不得在准备时预留。**

优先级固定为：host 错误与停止 → 当前事务完成 → mailbox/control confirmation 或 indication
→ 内部 EAPOL → CARD_INT/RX 扫描 → 普通 staged TX。完成与 CARD_INT 同时到达时，沿既有
acknowledge/latch 顺序先记录两者，再由 owner 决策，不能因为后继 DMA 已准备好而绕过接收或
控制路径。任意时刻仍至多一笔 host transaction 在飞。

### 3.5 失败、取消与关闭

- **提交前取消**：丢弃 staged/prepared 内容，只归还其 token 一次；`PreparedDma` 通过
  "从未交给设备"的安全回收入口（`PreparedDma::complete_without_device` / `into_cpu_buffer`）
  返回。
- **提交拒绝**：取回原始 Prepared backing 与批次所有权，按可重试/终止错误策略处理。
- **提交后取消或停机**：先停止新准备，再按现有 host abort/quiesce 协议终结在飞事务；
  只有在 `CompletedDma` 可安全恢复的证明成立后复用 backing，否则 quarantine。
- shutdown 与初始化失败**不得**在设备仍可能访问时 drop 或复用 DMA。

停机语义已核对：**核心侧不做中止**。`advance_once` 在 `io.pending` 非空时先返回
`WaitForInterrupt`，`drive_shutdown()` 只在在飞写按原路径结算之后运行，与
`docs/design/unified-sdio-aic8800.md` 的描述一致，正式文档无需改动。
（`drive_shutdown()` 内部那段 `io.pending` 分支因此不可达，留待下一次触及该函数时清理。）
`rdif/owner/progress.rs::shutdown()` 的 abort 是适配层的最后手段，跑在核心已结算之后，不冲突。

### 3.6 正式架构文档同步

`docs/design/unified-sdio-aic8800.md` 需要同步事务所有权、双槽阶段、commit 门控、IRQ 与
取消/关闭语义。**不得**把工作树的探针过程、个人文件或未经测量的收益写进正式文档。

## 4. 关键文件

| 关注点 | 文件 |
| --- | --- |
| 驱动状态机与准备/提交 | `drivers/net/aic8800/src/device/owner.rs`、`device/model.rs`、`device/data_plane.rs`、`device/progress.rs` |
| 单 owner 调度、DMA 槽与失败回收 | `drivers/net/aic8800/src/rdif/owner/progress.rs`、`rdif/owner/operation.rs`、`rdif/owner/output.rs`、`rdif/device/endpoints/startup.rs` |
| DMA 活动前缀公共契约及消费者迁移 | `memory/dma-api/src/owned.rs`、`drivers/blk/sdmmc-protocol/src/sdio/io/transfer.rs` 与必要的 SD/MMC host、SDHCI ADMA2 路径 |
| TX 到达唤醒 | `net/ax-net/src/queue_runtime/executor/mod.rs`，复用 `queue_runtime/state.rs` 与 `notify.rs` 的固定 owner/sticky notification 机制 |
| 正式架构 | `docs/design/unified-sdio-aic8800.md` |
| 测试增强入口 | `drivers/net/aic8800/src/device/data_plane.rs` 的批次/credit/cancel 用例、`memory/dma-api` 单元测试、`drivers/blk/sdmmc-protocol` 的 SDIO 生命周期测试 |

## 5. 已实现的部分

### 5.1 本轮已落地（第一阶段）

- **协议核心的 staging 槽**：`DataPlaneState.staged_tx` 保存「在飞写期间已经成形好的下一笔」，
  `active_tx` 仍是请求路径读得到的「总线槽」。`prepare_next_transmit_inner` 在总线槽空闲时先把
  `staged_tx` 提升进去、否则才成形；`advance_once` 在 `io.pending` 是数据写且状态为 `Ready` 时
  先做一次有界 `stage_next_transmit()` 再返回 `WaitForInterrupt`。核心不产生第二笔提交。
- **成形窗口的限制**：只有已经交给总线的写才允许有后继提前成形，且成形上限扣掉在飞写将要花掉的
  包数，同一份 credit 不被两笔写重复认领（理由见 §3.1、§3.4）。
- **token 归属**：`take_active_write_tokens()` 同时回收两个槽，取消、停机、失败三条路径各归还一次。
- **旋钮与探针**：`aic,tx-prepare-ahead`（0/1，默认 1）供同会话 A/B；probe 的 `chain` 行新增
  `staged ready/missed`。
- **`dma-api` 活动前缀契约**：`CpuDmaBuffer::capacity()`、`prepare_prefix(len)`、
  `DmaError::InvalidActiveLength`，`PreparedDma`/`InFlightDma`/`CompletedDma`/`QuarantinedDma`
  全链路携带活动长度；`prepare_for_device()` 语义不变。消费方 `sdmmc-protocol`、`sdhci-host`、
  `dwmmc-host` 的 `DmaError` 穷尽匹配已补齐。
- **正式架构文档**：`docs/design/unified-sdio-aic8800.md` 增「前车成形」条款与 token 回收范围。

本轮**尚未**实现 owner 侧准备好的 DMA backing：核心仍把成形写的字节交给请求路径复制，
`ActiveOperation::submit()` 仍按每笔写分配 `CpuDmaBuffer`。`dma-api` 的活动前缀目前还没有调用方，
它是下一步双槽的前提。

### 5.2 重构可以复用的既有能力

以下是继续实施时的边界，本身不等同于双缓冲：

- `AicDevice::advance()`、`AicOwner::advance_with_cause()` 与 `ActiveOperation::advance()`
  都是 IRQ/截止时间驱动的有限步骤状态机；
- `AicOwner` 是固定 CPU 上唯一的 `SdioCard`、host、`AicDevice`、card IRQ 与 DMA 完成回收所有者；
- hard IRQ（`AicHardIrq::handle_irq()`）只读取、确认硬件并写入 `IrqLatch`，不做协议与 DMA 工作；
- `QueueNotification`、`PollGroupState` 与 `WakeReason::Transmit` 已提供 sticky 的固定 owner 通知；
- RDIF SPSC 环以 move-only `DmaBuffer`、`RxCompletion` 与完成 token 传递所有权；
- TX 聚合、credit 本地记账与薄池等待、完成前交帧（`continue_transmit_pipeline`）与
  RX/control 优先级已在工作树中；
- `CpuDmaBuffer -> PreparedDma -> InFlightDma -> CompletedDma`（以及 `QuarantinedDma`）
  已经表达"设备访问期间 CPU 不可复用"的基础生命周期；
- `SdioDmaSubmitError::into_parts()` 已经把被拒请求的 backing 原样归还给调用方。

## 6. 待实现的部分

- **owner 侧准备好的 DMA backing**：`ActiveOperation::submit()` 仍按每笔写分配并填充
  `CpuDmaBuffer`；没有第二个 backing，也没有 owner-prepared 的提交接口
  （`SdioRequestKind` 没有相应变体，`AicAction` 没有「把下一笔的字节交给 owner 但不提交」的语义）；
- **两槽回收/隔离路径**：提交拒绝、取消、shutdown、abort 失败与 host quiesce 失败的 DMA
  回收目前只对单槽成立；
- **credit 在准备后变化时 staged 批次的重建/裁剪**；
- **双槽专用确定性 host fake 测试**；
- **同会话串行 / 逻辑 staged / DMA 双槽三臂板测**。

`dma-api` 的活动前缀契约已经就位，但**尚无调用方**；它由上一项使用。

## 7. 实施顺序与进度

1. ✅ **固定现状**：保存工作树与既有镜像映射，不回退本阶段之前的探针、credit、聚合、
   续写或运行时通知改动。
2. ✅ **统一 shutdown 语义**（§3.5）：核对结论是**核心侧停机不做中止**——
   `advance_once` 在 `io.pending` 非空时先返回 `WaitForInterrupt`，`drive_shutdown()` 只会在
   在飞写按原路径结算之后运行，与 `docs/design/unified-sdio-aic8800.md` 的描述一致；
   正式文档不需要改。（`drive_shutdown()` 内部那段 `io.pending` 分支因此不可达，
   留待下一次触及该函数时清理。）
3. ✅ **核心状态拆分与逻辑 prepare-ahead**：新增 `staged_tx` 槽与 `stage_next_transmit()`，
   成形搬到在飞窗口内，token 归属与三条回收路径同步更新，附 `aic,tx-prepare-ahead` 旋钮与
   `staged ready/missed` 计数。
4. ✅ **`dma-api` 活动前缀**：`prepare_prefix` 与全链路活动长度，消费方的 `DmaError` 匹配已补。
5. ⏳ **owner DMA 双槽**：owner 持有两个受上界控制的 ToDevice backing，准备态由固定 owner 写入，
   完成并 quiesce 后回收为下一准备槽；同时把 `take_write_bytes()` 的字节复制与 DMA 填充
   一并移进在飞窗口。
6. ⏳ **同步正式架构文档**（§3.6）：本轮已补「前车成形」条款，双槽落地后需再补事务所有权与
   取消语义。

第 5 步仍以第 3 步的板测结论为前提：先用有界观测证明逻辑槽确实降低了提交空档且没有造成
RX/control 回归，再扩大 DMA 公共接口的使用面。

## 8. 验证与交付门槛

### 8.1 宿主侧

修改 Rust 后运行 `cargo fmt`；定向静态检查使用 `cargo xtask clippy --package aic8800`、
`--package ax-net` 以及实际受影响且任务工具支持的 DMA/SDIO 包；白名单包使用
`cargo xtask test --since dev`。不得用原生 `cargo clippy` / `cargo test` 替代项目入口。

### 8.2 确定性测试

至少覆盖：

- A 在飞时只能形成 B，且 host **绝不**出现第二笔提交；
- completion 与 CARD_INT 同时到达时先服务 RX/control；
- credit 在 prepare 后改变时不会超额提交；
- staged、prepared、in-flight、completed 各阶段 token 恰好释放一次；
- 聚合前缀/块对齐长度正确，准备槽无数据时记为 miss；
- 提交拒绝原样返回同一 DMA backing；
- 提交前取消；提交后 abort/quiesce 之后才回收；
- 无法证明硬件停止访问时 backing 进入 quarantine；
- RX、mailbox、EAPOL 与控制事件不会被普通 staged TX 越过。

用可控 fake host 证明协议顺序，不把它描述为真实中断/缓存验证。

第一阶段已覆盖「A 在飞时只能形成 B，且 host 绝不出现第二笔提交」「credit 在 prepare 后改变时
不会超额提交」与「staged/in-flight 的 token 恰好释放一次」三条；其余各条要等 owner 侧双槽
落地后才有对应实现。

### 8.3 实体板卡

串行基线、逻辑 staged、DMA 双槽三臂必须在**同一热点会话、同一内核、固定姿态、代理关闭**
且保留健康参考臂的条件下完成。第一阶段的可用观测量是 probe 的 `chain` 行
（`staged ready`/`missed`，须满足 `ready + missed = tx_writes`）、`chain idle`、`period avg/max`
与 `size blk`/`bytes=` 分布；owner 侧双槽落地后还需补 `completion_to_commit`、DMA 获取/回收
计数与 token/DMA 守恒。两种排列由 `aic,tx-prepare-ahead` 在同一内核下切换。
**跨会话吞吐不作为收益证据。**

### 8.4 回滚

保留可编译的串行提交路径与前一可启动镜像；任一状态机、DMA API 或板测门槛失败时不得默认
启用双槽。阶段完成前不改变默认策略，也不把未测量的理论上限写成实际收益。

## 9. 实板结果（2026-09-29）

四张镜像出自同一内核（`starryos.bin` sha256 `57ac05ff…`）与同一基座（`…_q64_20260928.img`），
两两之间只差 DTB 里的一个属性。原始日志见 `nowait-board-20260929.log`、
`ahead-a1/ahead-b1/ahead-a2-board-20260929.log`、`ahead1-weak/ahead1-rerun-board-20260929.log`。

**① 两个旋钮的机制足迹都成立且可量。** `staged ready + missed = tx_writes` 与
`chain` 七项之和 = `tx_writes` 逐窗精确闭合（79/54/98 窗零违例）。

| 指标（上行窗口） | 成形关 | 成形开 | 成形开 + 无等待 |
| --- | ---: | ---: | ---: |
| `staged ready` 占完成笔数 | 0% | 86.8 / 87.8% | 88.7% |
| `chain idle` | 17.6% | 10.0 / 10.0% | 6.3 / 7.1% |
| `write credit` 3-6 档份额 | 6.4% | 4.8 / 4.9% | **20.2%** |
| 每笔写字节 | 24.5 KB | 21.4 / 21.8 KB | **18.6 KB** |
| 写周期 | — | 4483–4718 µs | **3914/3984 µs** |
| 发射相位 deadline 等待占比 | — | 11.0–13.7% | **8.2/10.5%** |
| 总线腿占墙钟 | 36.1% | 35.9 / 36.0% | 35.8% |
| 上行 iperf3 | 38.0 | 33.4 / 36.9 / 37.2 / 36.4 | 38.0 / 35.3 |

**② 但没有一条把总线占空比推上去。** 三臂的总线腿占比都是 35.8–36.1%；上行吞吐的臂间差
落在运行间散布之内（成形关只有一次运行；双向 TX 里成形开的臂反快 2.4%；合池 33.20(n=7) 对
33.40(n=2)；逐秒 Mann-Whitney p = 0.081）。**结论是判不出次序，而不是某一臂更好。**

**③ credit 等待的构成已查清。** `aic,tx-credit-wait-us` 只作用于薄池分支（`credits 3..=7`）；
**保留量分支（`credits <= 2`，`IO_RETRY` 200 µs）占 credit 等待约 78%，旋钮管不到**，
遥测/邮箱/启动/SDIO 寄存器重试等同样不受影响。所以那 16.1% 不是空转——等待在等固件池回填，
去掉它只是把每笔写从 20.9 KB 降到 18.6 KB。

**④ 两条实现问题（未修，是下一步的前置）：**

1. `stage_next_transmit` 的 `limit = aggregate_limit().saturating_sub(in_flight).max(1)`
   把减法放在策略上限**之外**，credit 充足时把成形上限从 32 帧压到 `32 − s_prev`；
   正确写法是 `min(credits - 2 - in_flight, 32)`（credit 紧时两者等价）。判别性证据是
   缺口只出现在"照缓存 credit 直接发出"的带宽带里并随 credit 增大而扩大，但要证否需补
   逐笔 `s`/`s_prev` 插桩。
2. 探针 `supply core/none` 看不到 `staged_tx`，成形开启的臂 `none` 虚高（24–26% 对 15.9%），
   该量不可跨臂比较。

## 10. 阶段结论

本轮交付的是异步双缓冲方案的**第一段**：**在飞窗口内的成形（前车成形）已落地并经实板验证，
`dma-api` 的活动前缀契约已就位，无等待对照臂已量清 credit 等待的构成**。

结论是**机制兑现、收益未兑现**：成形把 `chain idle` 压低约 7–10 个点、写周期缩短约 12%，
但两者都没有改变总线占空比（36%）与吞吐。限制项不在被搬动的那段时间里——
上行非在飞时间的最大单项仍是"没有帧可发"（栈侧供给节奏），而 credit 等待里 78% 属于旋钮
够不到的保留量分支。

**因此不建议继续在这两个旋钮上调参，也不建议此时进入 owner 侧 DMA 双槽**（§7 第 5 步）：
双槽的收益前提是"空档被 CPU 准备工作占着"，而本轮证据指向的是相反的结论。下一步按性价比是：

1. **补一个零代码对照臂**：把现成的 `attrib3` 镜像当第三臂，在同一热点会话里与 `ahead0`/`ahead1`
   用相同用例序列交错 2–3 次。`ahead0` 与 `ahead1` 都含本轮重构，B 不是它的对照——
   这一步能分清"会话差异"与"重构公共路径的非等价改动"。
2. **修上面两条实现问题**后重跑同样的 A/B，看写长度差是否消失。
3. 两项都做完、成形仍无总线层面收益时，再决定是否把 staging 槽降级为默认关闭。
   即便吞吐不动，它在 `chain idle` 上留下的差是机制账，不是收益账，两者要分开记。
