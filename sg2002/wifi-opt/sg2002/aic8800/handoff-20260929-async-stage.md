# AIC8800 异步双缓冲重构交接

本文件面向接手这条工作线的后续会话或 agent。方案本身见
`async-double-buffer-delivery-20260929.md`（原始规划记录为 `async-sdio-session-original-plan.txt`），
本文只讲**怎么接、从哪接、接的时候不能动什么**。

## 1. 工作线位置

| 项 | 值 |
| --- | --- |
| 分支 / 工作树 | `sg2002/wifi-opt` / `wt-sg2002-wifi-opt` |
| 工作树提交状态 | HEAD `a6633f3f6`；本阶段未提交、未推送、未创建 PR |
| 工作树改动 | 30 个被追踪文件（前轮的探针、credit、聚合、续写、运行时通知，本轮的前车成形、活动前缀与文档），全树 `+2677 −280` |
| 不纳入 | `.claude/`、`.ocr/`；`AGENTS.md`、`CLAUDE.md` 已被改动但**不提交、也不回退** |
| 本轮触及的被追踪文件 | `drivers/net/aic8800/src/device/{owner,data_plane,progress,probe}.rs`、`.../rdif/{owner/progress,device/endpoints/device}.rs`、`drivers/ax-driver/src/net/aic8800/fdt.rs`、`memory/dma-api/src/{owned,def}.rs`、`memory/dma-api/tests/test.rs`、`drivers/blk/{sdmmc-protocol/src/sdio/init/mod.rs,sdhci-host/src/dma/mod.rs,dwmmc-host/src/dma/mod.rs}`、`docs/design/unified-sdio-aic8800.md` |

工作树里已有的探针、credit、聚合、完成前交帧与运行时通知改动是**继续实施的前提**，
不得清理或回退；本阶段也没有回退它们。

## 2. 原始要求

对 AIC8800 驱动层做较大范围重构，让 CPU 侧的构帧、聚合、DMA 填充与 cache publish 与
SDIO 传输重叠，而不是排在事务完成之后。范围明确选择"直接做到 DMA 双缓冲"。

硬边界：`drivers/blk/sdhci-host/README.md` 规定传统 SDHCI `queue_depth = max_submit_batch = 1`，
`drivers/blk/sdmmc-host/src/lib.rs` 按单事务建模，`drivers/blk/sdmmc-protocol/src/sdio/io/mod.rs`
以唯一 `active_io_request_id` 拒绝第二笔 IO 请求。所以"双缓冲"是
**一个 host 事务在飞 + 一个 CPU-owned 后继槽**，不是两笔 CMD53 并发。

## 3. 方案索引

| 内容 | 位置 |
| --- | --- |
| 分层与所有权、状态机、DMA 前缀契约、提交门控、生命周期 | `async-double-buffer-delivery-20260929.md` §3 |
| 关键文件表 | 同上 §4 |
| 实施顺序与验收门槛 | 同上 §7、§8 |
| 规划阶段的原始记录（含上下文与验证清单） | `async-sdio-session-original-plan.txt` |
| 进度依据（第四阶段实测读数） | `stage-delivery-20260930.md` §3、`aic8800-async-optimization-plan.md` §9 |

## 4. 当前代码状态

### 4.1 本轮已落地

- **协议核心的 staging 槽**：`DataPlaneState.staged_tx` 保存「在飞写期间已经成形好的下一笔」，
  `active_tx` 仍是请求路径读得到的「总线槽」；`advance_once` 在 `io.pending` 是数据写且状态
  为 `Ready` 时先做一次有界 `stage_next_transmit()`，核心不因此产生第二笔提交；
- **成形窗口的限制**：只有已经交给总线的写才允许有后继提前成形，成形上限扣掉在飞写将要花掉的
  包数（同一份 credit 不被两笔写重复认领）；
- **token 归属**：`take_active_write_tokens()` 同时回收两个槽，取消、停机、失败三条路径各归还一次；
- **旋钮与探针**：`aic,tx-prepare-ahead`（0/1，默认 1）；probe 的 `chain` 行新增
  `staged ready/missed`；
- **`dma-api` 活动前缀契约**：`CpuDmaBuffer::capacity()`/`prepare_prefix()`、
  `DmaError::InvalidActiveLength`，`PreparedDma`/`InFlightDma`/`CompletedDma`/`QuarantinedDma`
  全链路携带活动长度；消费方 `sdmmc-protocol`、`sdhci-host`、`dwmmc-host` 的 `DmaError`
  穷尽匹配已补；
- **正式架构文档**：`docs/design/unified-sdio-aic8800.md` 已补「前车成形」条款与 token 回收范围。

### 4.2 可以直接复用的既有能力

- `AicDevice::advance()`、`AicOwner::advance_with_cause()`、`ActiveOperation::advance()`
  均为 IRQ/截止时间驱动的有限步骤状态机；
- `AicOwner` 是固定 CPU 上 `SdioCard`、host、`AicDevice`、card IRQ 与 DMA 完成回收的唯一所有者；
- hard IRQ 只确认、快照并写入 `IrqLatch`，不做协议与 DMA 工作；
- `QueueNotification`/`PollGroupState`/`WakeReason::Transmit` 已提供 sticky 固定 owner 通知；
- TX 聚合、credit 本地记账与薄池等待、完成前交帧（`continue_transmit_pipeline`）、
  RX/control 优先级均已在工作树中；
- `CpuDmaBuffer -> PreparedDma -> InFlightDma -> CompletedDma`/`QuarantinedDma` 已表达
  "设备访问期间 CPU 不可复用 backing"；
- `SdioDmaSubmitError::into_parts()` 已把被拒请求的 backing 原样归还。

### 4.3 尚未具备

- owner 没有 prepared DMA 槽，`ActiveOperation::submit()` 仍按每笔写分配 `CpuDmaBuffer`
  并 `copy_from_slice_cpu()`；没有第二个 ToDevice backing，也没有回收/隔离路径；
- `take_write_bytes()` 的字节复制仍在完成之后（只在飞窗口内搬走了「成形」一段）；
- `SdioRequestKind` 没有 owner-prepared DMA 的提交变体，`AicAction` 也没有
  「把下一笔的字节交给 owner 但不提交」的语义；
- 没有双槽专用的 fake host 测试，也没有任何双槽观测量。

### 4.4 已知缺口

- `memory/dma-api` 的集成测试在本树跑不起来（既有问题）：默认 feature 下 `tests/test.rs`
  依赖受 `pool` 门控的 `contiguous_buffer_pool`；加 `--features host-test` 后能编译，
  但链接缺 `__SpinOps_acquire/release`（`ax-sync` 需要测试侧提供宿主 `SpinOps` 实现，
  `memory/buddy-slab-allocator/tests/common/` 有先例）。该 crate 也不在
  `scripts/test/std_crates.csv`，`cargo xtask test` 不会选中它。本轮新增的前缀测试已用临时
  provider 跑通并做过变异验证，但在补齐 provider 与白名单之前不构成项目入口下的证据。

## 5. 实施顺序与进度

1. ✅ **固定现状**：保存工作树与既有镜像映射，不回退既有改动。
2. ✅ **统一 shutdown 语义**：核对结论是核心侧停机不做中止（`advance_once` 在 `io.pending`
   非空时先返回 `WaitForInterrupt`），与正式架构文档一致，无需改文档。
3. ✅ **核心状态拆分与逻辑 prepare-ahead**：新增 `staged_tx` 槽与 `stage_next_transmit()`。
   **已实板**：机制闭合（`staged ready` 86.8/87.8%，逐窗 `ready+missed = tx_writes`），
   `chain idle` 17.6% → 10.0%，但总线腿占比与吞吐均无变化。
4. ✅ **`dma-api` 活动前缀**：`prepare_prefix` 与全链路活动长度（仍无调用方）。
5. ✅ **无等待对照臂**：零代码改动，只改 DTB 一个属性。已量清 credit 等待的构成——
   旋钮只作用于薄池分支（`credits 3..=7`），**保留量分支（`credits <= 2`）占约 78%**；
   去掉等待后每笔写 20.9 → 18.6 KB，占空比与吞吐不动。
6. ⏸ **owner DMA 双槽**：**暂不进入**。双槽的收益前提是"空档被 CPU 准备工作占着"，
   本轮证据指向相反结论（空档最大单项是"没有帧可发"，credit 等待的 78% 旋钮够不到）。
   重启这一步的条件见下。
7. ⏳ **同步正式架构文档**：双槽落地后补事务所有权与取消语义。

**先做这两件事，再谈第 6 步：**

1. **补一个零代码对照臂**：把现成的 `attrib3` 镜像当第三臂，同一热点会话内与 `ahead0`/`ahead1`
   用相同用例序列交错 2–3 次。`ahead0`/`ahead1` 都含本轮重构，B 不是它的对照，
   这一步能分清"会话差异"与"重构公共路径的非等价改动"。
2. **修两条实现问题**（详见 `async-double-buffer-delivery-20260929.md` §9 第 ④ 条）：
   `stage_next_transmit` 的减法位置（应写 `min(credits - 2 - in_flight, 32)`）、
   以及探针 `supply` 看不到 `staged_tx` 的口径缺陷。修完重跑同样的 A/B，
   看写长度差是否消失。

## 6. 所有权与生命周期不变量

实施时任何一步都不得破坏下列不变量：

- `AicOwner` 独占 `SdioCard`、host、card IRQ、DMA 完成回收与 `AicDevice`；
- 任意时刻**至多一笔** host transaction 在硬件中飞；
- hard IRQ 只确认/快照/发布事件，不构帧、不分配、不接触 DMA payload；
- credit 只在 `Staged -> Submitted` 的提交门控处由权威状态重新检查并消费，
  准备阶段不得预留信用快照；
- CARD_INT、错误、mailbox/control、内部 EAPOL 与 RX 扫描都优先于普通 staged TX；
- 每个 TX token 恰好完成或归还一次；
- `PreparedDma`/`InFlightDma` 期间 CPU 不可写入或复用 backing；只有 host 完成或 abort 后
  确认 quiesce 才能回收，否则 quarantine；
- `AicDevice` 不持有 `DeviceDma`、操作系统锁、任务或执行器——DMA 能力只出现在
  RDIF owner/SDIO 适配边界。

## 7. 验证与交付门槛

宿主侧必须通过项目入口运行格式化、定向 clippy 与增量标准库测试。相关白名单包包括
`aic8800`、`rdif-eth`、`ax-net`、`sdmmc-protocol`；`dma-api`、`sdmmc-host`、`sdhci-host`
是否进入对应任务矩阵需先以 `cargo xtask` 支持为准，不能直接用原生 Cargo 命令替代项目入口。

**构建前提（本轮踩过）**：本机取不到 `raw.githubusercontent.com`，而 `aic8800` 的 build script
默认从那里下载固件，任何触及该 crate 的 cargo 调用都会**一直挂住**。构建前必须先指向已校验的缓存：

```sh
export AIC8800_FIRMWARE_DIR=/home/asta/tgoskits/tgoskits/.sg2002-build/firmware
```

本轮实际跑过：`cargo fmt --all`（干净）；`cargo xtask clippy --package aic8800`
（base / `rdif` / `host-test` 三组全过）、`--package sdmmc-protocol --package sdhci-host
--package sdmmc-host --package ax-net`（15 项全过）、`--package ax-driver`（51 项全过）；
`cargo xtask test --since dev`（全过）；`cargo test -p aic8800 --features host-test`（161 + 1）。

确定性测试清单与板测三臂要求见 `async-double-buffer-delivery-20260929.md` §8。
真实板卡是 DMA cache、IRQ 时序与 owner 调度的最终证据；没有完成同会话 A/B、机制计数闭合
和非退化检查之前，不应声称"双缓冲已提升 SDIO 效率"。

## 8. 接手注意事项

- **收益口径**：双缓冲回收的是"成形 + DMA 准备 + 提交"这段固有延迟（每笔写约 0.5–0.6 ms，
  见 `stage-delivery-20260930.md` §3.5）。credit 自选等待占上行 16.1% 墙钟，属于固件缓冲池
  策略，必须先用同会话 `aic,tx-credit-wait-us=0` 对照单独归因，不能与双缓冲混作一个变量。
- **跨会话数字不作证据**：同一热点会话内的 A/B 才有效；换会话后 PC 侧热点状态是第一变量。
- **临时探针**：工作树里的板测探针属临时改动，开 PR 前必须移除，移除记录写进
  `aic8800-optimization-tracker.md`。
- **既有遗留问题**（与本轮无关，不要顺手改）：`device/control.rs::scan_command` 的 payload
  缓冲边界、双向偶发断链、ARP 300 秒邻居项事件、信道 survey 读数源、帧年龄。
- **分支纪律**：本工作线只迭代 `sg2002/wifi-opt`，新建或切换分支前必须先确认。
