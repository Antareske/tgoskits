# Code Review: sg2002-wifi-opt AIC8800 性能与重构评估

**Date**: 2026-09-27  
**Reviewers**: @principal-1, @principal-2, @quality-1, @quality-2  
**Mode**: Full (with discourse)

## Verdict

**NEEDS DISCUSSION**

驱动重构可行，但当前不应以重写 `ax-net`、增加驱动线程或并发提交 SDIO 命令为主线。最优先的工作是修正普通数据的 firmware confirmation 策略和性能统计口径；随后再决定聚合、无线能力、复制/DMA 流水线的投入顺序。具体收益必须由同会话板上 A/B/A 决定。

## Blockers

无。本轮是静态架构评估，不是待合并补丁；下列项目属于下一阶段应先处理或验证的工作。

## Should Fix

### 1. 普通数据逐包请求了未被消费的 firmware TX confirmation

**Flagged by**: @principal-1, @principal-2, @quality-1, @quality-2  
**Location**: `drivers/net/aic8800/src/protocol.rs:205-215`; `drivers/net/aic8800/src/device/data_plane.rs:377-385`

Rust 对每个 Ethernet TX 固定写入 `hostid = 0x8000_0001`。厂商 FDRV 只对特定 management 与 EAPOL/WAPI 帧设置 bit 31 并分配可关联的 ID；普通数据明确使用 `hostid = 0`。当前 Rust 在 SDIO write completion 后已归还数据面 token，收到 `DataConfirmation` 仅打印 trace，没有状态、重试或资源消费者。

运行时计数也与该机制高度相关：一个窗口中 `3849 rx_parsed - 468 rx_done = 3381`，接近 `3464 tx_submit`；另一个窗口中 `3912 - 489 = 3423`，几乎等于 `3434 tx_submit`。这不是分类计数意义上的证明，但足以把它列为数据面 P0。它很可能制造了大量 FIFO、CARD_INT、ReceiveCount/Data、scan 收尾与 rearm 工作。

先拆分 typed counter，再做窄 A/B：普通 user data 使用 `hostid = 0`；EAPOL/management/WAPI 暂时保留 confirmation。若特殊帧确实需要结果，应使用唯一 correlation ID、明确消费者与超时，而不是共享魔数。记录关联/四次握手稳定性、吞吐、丢包/重传、credit、数据/控制读、IRQ 与各类 confirmation。

### 2. 已被板测否定的 RX defer 仍是默认行为

**Flagged by**: all reviewers  
**Location**: `drivers/net/aic8800/src/device/data_plane.rs:21-35`; `drivers/net/aic8800/src/rdif/device/endpoints/device.rs:63-78`

默认仍为 1 ms，注释仍声称它能安全合并小帧。配对试验已显示 1 ms 无收益且 `frames/read` 略降，3 ms 则把上行从 17.2 Mbps 降到 1.57 Mbps，并把 firmware-side transfer 放大约十倍。

默认值应改为零；非零值只保留为显式实验配置，并在启动日志打印最终生效值。新发现不会推翻该负面 A/B，只会推翻“TCP ACK 每约 590 us 滴流，所以窗口没有余量”的旧机制解释。3 ms 的灾难退化更可能包含延迟排空自请求 confirmation 造成的 firmware backpressure。

### 3. 性能探针的分子、分母和观测开销均不足以支持下一轮决策

**Flagged by**: @quality-1, @quality-2, with agreement from both principals  
**Location**: `drivers/net/aic8800/src/device/probe.rs:259-303`; `net/ax-net/src/queue_runtime/executor/probe.rs:62-159`

聚合后 TX `pkts` 实际递增的是 CMD53 write 次数；RX `frames` 又在分类前统计所有 parsed item，包含 Ethernet data、`DataConfirmation`、mailbox confirmation/indication 和 firmware print。因此历史 `per_pkt`、`frames/read`、ACK 速率和 transaction/frame 推导需要重算。

至少拆分 `tx_frames`、`tx_writes`、`rx_data_frames`、`tx_data_confirmations`、LMAC confirmations/indications、firmware prints、data/control reads、scans、tail-empty reads、IRQs 与 credit。常驻 atomic/timestamp/info 日志还需 `perf-probe` feature 或 no-op production 实现，并做 probe-on/probe-off 配对，避免用观测开销决定 3%--10% 级优化。

### 4. SDHCI 正常命令路径无条件读取整组诊断 MMIO

**Flagged by**: all reviewers  
**Location**: `drivers/blk/sdhci-host/src/command.rs:338-390`; `drivers/blk/sdhci-host/src/command.rs:413-448`

`program_command()` 每次都调用 `log_status("issued")`，而寄存器读取发生在 `log::debug!` 之前。关闭 debug 只省格式化，不省 present state、IRQ、clock、power、host control、reset 等 MMIO。代表性窗口超过 5,500 笔事务/2 s，这项固定成本被直接放大。

在任何寄存器读取前按 issued 路径的日志级别短路；错误和超时路径继续保留完整状态采样。用 fake-MMIO/read-counter 测试“debug 关闭时 issued 零诊断读、错误路径仍采样”。这是可与 confirmation A/B 并行落地的低风险项。

### 5. 无线 capability 配置需要类型化、遥测和受控启用

**Flagged by**: @principal-1, @principal-2, @quality-2  
**Location**: `drivers/net/aic8800/src/lmac.rs:289-305`

当前 `ME_CONFIG` 把 `phy_bw_max` 写成 80 MHz，但 HT capability 只有 `0x0001`，没有 20/40、SGI/STBC，VHT/HE 全零；同时缺少实际 negotiated width/MCS/retry 证据。它很可能限制空口上限，但上下行不对称和同镜像波动不能单独证明因果。厂商树中的一部分动态兼容检查位于 `#if 0`，也不能当成现成的安全 gate。

把裸 112 字节数组重构为 typed `MeCapabilities`，由 `ChipProfile + observed firmware/version + board policy` 构造，并做 vendor layout golden test。行为实验按 HT40/SGI、VHT、HE 分阶段进行；每轮记录真实带宽、MCS、retry、channel 和 credit。同会话 A/B/A 是必要条件。

### 6. TX/RX 多级复制应重构为 move-only batch 和可回收 owned DMA

**Flagged by**: all reviewers  
**Location**: `drivers/net/aic8800/src/rdif/owner/output.rs:82-90`; `drivers/net/aic8800/src/device/owner.rs:57-78`; `drivers/net/aic8800/src/rdif/owner/operation.rs:81-203`; `drivers/net/aic8800/src/rx.rs:108-247`

TX 依次经历 RDIF buffer 到 `Vec`、逐帧 wire `Vec`、聚合 append、`wire_bytes()` 整批 clone/pad、`new_zero()` owned DMA 和再次 copy；RX 也丢弃 completed DMA ownership，再经历 parser、decapsulation 与 runtime buffer 复制。

分阶段处理：

1. 把 token/credit/retry 元数据与 wire storage 分离，状态显式化为 `QueuedTx -> FramedTx -> CreditGrantedBatch -> InFlightSdio -> CompletedBatch`；finalize 后 move buffer，不再从 `&self` clone 整批。
2. encoder 直接 append 到 active aggregate，只有整笔末尾做 512-byte padding；RX parser 先返回 completion backing 上的 borrowed views，最终 Ethernet payload 至多一次落地复制。
3. adapter 使用一笔 in-flight 加一笔 CPU-owned/prepared，并让 `CompletedDma` 回收到 owner-local pool。

现有 `PreparedDma::len()` 等于 backing 长度，CMD53 又要求精确 transfer shape，因此首版应使用按 512-byte block count 的 size-class pool。不要通过裸指针绕过 typestate；只有池化仍被测为不足时才考虑共享 `PreparedDmaRange` API。复制收益尚未分段计时，不能承诺 2x，单独更可能是低双位数以内。

### 7. 维护文档与性能准入标准需要按新证据更新

**Flagged by**: all reviewers  
**Location**: `www/sg2002/aic8800/aic8800-async-optimization-plan.md`; `www/sg2002/aic8800/aic8800-bottleneck-analysis.md`; `www/sg2002/aic8800/aic8800-optimization-tracker.md`

需要撤回或降级三类陈述：把混合 `rx_frames` 当 TCP ACK；把 vendor 双线程当 SDIO 并行；把 capability 或流水线化写成保证达到约 40 Mbps 的因果结论。保留 K=4、RX defer、transaction timing 等实测结果，但明确区分代码事实、强相关推断与待测收益。

每个实验写清进入/退出判据，至少包含同镜像 A/B/A、探针模式、关联/MCS/channel、吞吐、双向公平性和尾延迟。StarryOS UDP 约 1 Mbps 时驱动长期拿不到第二帧，应继续作为 socket/协议栈/系统调用层问题单独调查，不用驱动重构解释。

## Suggestions

### 1. 在 confirmation 基线修正后再做聚合 K/byte sweep

当前 K=4 是唯一有大幅实测收益的优化：K=1 8.42 Mbps 到 K=4 18.7--19.5 Mbps。源码注释把 4 帧/6144 B 写成 vendor transport 上限并不准确：FDRV 默认 K=32、预分配 `1536*64`，BSP 才有独立的 4 帧常量。

先修 confirmation，再扫 K=4/8/16/32 与 6/12/24/48 KiB；记录实际 frames/write、credit 截断、RX latency、双向公平性和尾延迟。同时决定 byte 参数是硬容量还是 `flush_after_bytes` 软阈值，避免未来固定 staging 出现容量歧义。

### 2. RX tail-empty read 只作为后置窄实验

尾空读约有 1,075 次/2 s，理论上存在个位数百分比机会，但取消它会触碰 level IRQ closure、D80 `OTHER` ack 和 DC 双 function 不变量。先消除 ordinary confirmation 并重新基线；仍有充分机会时，再试“每 fact 一次 count+data 后原子 rearm_and_check，level 仍高则立即重调度”，并记录 tail-empty、rearm-pending 和 starvation。

### 3. ownership 收紧与 ADMA 描述符缓存只在主线稳定后处理

buffer-owning event/request 不宜长期保留深 `Clone`；随 move-only 状态重构删除。ADMA table/地址写入可在复用稳定后评估缓存，但只能在 DMA address、length、direction 完全相同时复用，优先级低于协议流量、MMIO guard 和 buffer move。

## Consensus & Dissent

### Topic: `ax-net` 是否是当前主瓶颈

四位 reviewer 一致认为不是。executor poll 约占墙钟 5%，owner 统计又与总线 transaction span 重叠；现有 budget、fixed CPU、atomic rearm/check 和单 owner 均是合理边界。170 us handoff 只覆盖会 park 的 DMA 操作，不能外推到多数 owner-polled CMD52。

### Topic: 最优先的数据面实验

交叉质询后，一致把 ordinary-data confirmation A/B 提升到 P0，并把 K sweep、RX tail closure 与大规模 DMA/runtime 重构后移。争议只剩确切收益幅度；所有 reviewer 都拒绝在板测前承诺倍率。

### Topic: capability 与 host-side transaction tax

两者不是互斥解释。confirmation policy 解决当前可避免的 host/firmware 事务税；typed capability 与实际 MCS/width 解决无线 ceiling。前者是最明确的低改动行为 A/B，后者是追近 Linux 58 Mbps 必须验证的独立主线。

## What's Working Well

- 固定 CPU、单 owner、单 SDIO in-flight、hard IRQ 只发布 fact、原子 rearm/check 的设计适合当前 host 与 level IRQ 约束。
- `CpuDmaBuffer -> PreparedDma -> InFlightDma -> CompletedDma` 已把 DMA ownership、abort 和 quarantine 语义编码进类型，应作为重构不变量。
- 本地 credit 记账和 K=4 聚合分别证明了“减少事务数”是有效方向；尤其 K=4 已带来约 2.3 倍提升。
- 维护记录保留了被推翻的实验，并纠正了 owner/wake 时间的旧解释，这种证据纪律很好。
- UDP 低吞吐时驱动空闲的证据已把问题与 AIC 数据面分离，避免跨层误判。

## Requirements Assessment

| Requirement | Status | Notes |
|---|---|---|
| 联合评估 AIC8800 与 `ax-net` runtime | Met | 已区分 executor CPU、owner span、SDIO transaction 与 IRQ handoff |
| 对照 Linux 基线与厂商源码 | Met | 已区分 FDRV/BSP、host 串行化、聚合和 capability 构造 |
| 静态定位瓶颈并区分证据等级 | Met with correction | 新发现要求重算 RX/ACK 相关历史口径 |
| 给出可实施的异步重构方案 | Met | 保留单 owner，以一在飞一准备、move-only batch、exact-size DMA pool 分阶段推进 |
| 给出可信的吞吐收益倍率 | Not currently possible | 同镜像上行曾有 3.4 倍波动，且缺 negotiated-rate 与 typed RX telemetry |

## Clarifying Questions

- 第一目标是稳定达到 40 Mbps，还是在相同关联档位追平 Linux 58 Mbps？两者需要不同的 capability、延迟与公平性取舍。
- 哪些 D80/DC firmware/profile 必须支持版本与 rate-control telemetry？不能假设所有变体共享一套 ME capability。
- 聚合默认值允许的单包尾延迟与双向公平性下限是多少？这决定 K=16/32 是否可以成为产品默认。
- 可复用 DMA 首版是否明确限定为 AIC adapter 内的 exact-size pool？若要扩展通用 active-range API，需要单独评审共享安全契约。
- 正式性能基线是否要求 probe-disabled 镜像？若允许常驻探针，需要给出可接受的观测开销上限。

## Individual Reviews

| Reviewer | High | Medium | Low | File |
|---|---:|---:|---:|---|
| @principal-1 | 3 | 4 | 1 | `reviews/principal-1.md` |
| @principal-2 | 6 | 3 | 0 | `reviews/principal-2.md` |
| @quality-1 | 4 | 2 | 1 | `reviews/quality-1.md` |
| @quality-2 | 4 | 3 | 1 | `reviews/quality-2.md` |

**Session**: `.ocr/sessions/2026-09-27-sg2002-wifi-opt/`  
**Discourse**: `rounds/round-1/discourse.md`
