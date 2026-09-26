# 终审：`probe/aic8800-supply`（TX 聚合实验）

审查范围：`git diff 77cc80819..HEAD`（14 文件，+1274/−50），重点为两个实质提交
`89ff23e4f`（聚合实现，**上板镜像包含**）与 `b677b55b1`（完成事件修正，**未上板**）。
依据与证据：`www/sg2002/aic8800/probev5-aggregation-review-20260925.md`、
`www/sg2002/aic8800/probev5-board-20260925.log`、厂商驱动
`/home/asta/tgoskits/LicheeRV-Nano-Build/osdrv/extdrv/wireless/aic8800/`。

## Verdict

**REQUEST CHANGES**（1 个阻塞项：聚合写的帧内对齐与固件遍历步长不匹配，导致 K−1 帧静默丢弃；
另有 1 个阻塞项：内部 EAPOL 写携带用户帧导致 DMA 缓冲永久泄漏。分支为实验分支，
本结论用于决定该改动能否进入正式分支，以及板端停住是否由此解释。）

## Blockers

### 1. 聚合写的帧内 512 对齐破坏固件的帧流遍历，第 2..K 帧被静默丢弃

**Flagged by**: @principal-2（Critical）、@principal-1（质证后 AGREE）、@quality-2（证据审计 CONNECT，并更正量化口径）
**Location**: `drivers/net/aic8800/src/protocol.rs:167-181`、`drivers/net/aic8800/src/device/data_plane.rs`
（`extend_active_write` 的 `extend_from_slice`）

固件按 `4 + align4(声明长度)` 步进遍历一笔写内的帧流，读到 `packet_len == 0` 即终止。厂商驱动逐帧只做
4 字节对齐（`aicwf_sdio.c:2214-2240` 写入 `[4B 头][28B desc][payload]`，`:2243-2247` 补零到
`TX_ALIGNMENT = 4`），整笔才在 `aicwf_sdio_aggr_send` 补齐到 512（`TXPKT_BLOCKSIZE`）；
我方 RX 解析器同样是 `4 + align_up(packet_len, 4)`、`packet_len == 0` 终止（`rx.rs:119`、`:174`）。

本改动把**每个帧**都补齐到 512 再首尾相接，于是固件读完第 1 帧后按步长落进该帧自己的零填充，
读到长度 0 即停止 —— **第 2..K 帧被丢弃，且 CMD53 正常完成、K 个 token 全部完成、K 个信用全部扣减、
日志无任何错误**。单帧写之所以一直正确，正是因为尾部零字节恰好充当了流结束标记。唯一例外是某帧
`raw_len` 本身为 512 倍数时下一帧恰落在步进点上（这类尺寸的聚合「看起来能用」）。

**与板端现象的关系**：机制在代码层面确定成立，且 K=1→K=4 的速率塌陷同步（周期 P4 纯上行 30.1 MB /
8.42 Mbps；本轮同用例 256 KB / 99.9–524 Kbit/s，塌陷 20–80 倍）。
但**因果链未闭合**：上行用例的交接帧数（189 帧，Tech Lead 自算）与 256 KB 对应段数（≈185）
比值约 1.0，quality-2 按字节账复算为约 283 帧（≈1.5×），两者区间归属不一致；且日志中
`Cwnd 0.00 Bytes` 说明设备侧 TCP_INFO 非真值，`Retr 0` 不可作为「无重传」的证据。
因此终稿不宣称「已证明是停住的唯一原因」，而把「按 4 字节对齐重建布局后重测」列为最省判别手段。

**修复方向**：聚合时逐帧只补 4 字节对齐、仅整笔末尾补 512（对齐厂商 `:2243-2247` +
`aicwf_sdio_aggr_send`）。注意 v3 的声明长度是 `28+payload`（`protocol.rs:177-185`），不覆盖补齐量，
因此不能靠改声明长度绕过。

### 2. 内部 EAPOL 写被追加用户帧，`Internal` 分支不回收 token → DMA 缓冲永久泄漏

**Flagged by**: @principal-1、@quality-1（两位独立提出，结论一致）
**Location**: `drivers/net/aic8800/src/device/data_plane.rs:437-438`（`consume_transmit_flow` 无条件
`extend_active_write`）、`:469` 与 `:470-478`（`TxCompletion::Internal` 两分支忽略 `extra_tokens`）

四次握手期间，若信用缓存不可用且队列里同时有用户帧与内部 M2/M4 帧，用户帧会被拼进内部写；
内部写完成后其 token 无人发布，对应 DMA 缓冲永久滞留在 `OwnerOutputs.tx_tokens`（池仅 32 块），
耗尽后 TX 路径只能拿到 `Again`。触发条件是连接期常态，无需竞态。本轮日志无 EAPOL，
故它不是本次停住的原因，但属静默资源泄漏，须在合并前修复。

## Should Fix

### 1. `TxBatch` 部分入队失败会丢 token 并令设备永久 `Failed`

**Flagged by**: @principal-2、@quality-1
**Location**: `drivers/net/aic8800/src/device/progress.rs:99-112`、`drivers/net/aic8800/src/tx.rs:34-40`

批次中途 `enqueue` 返回 `Err(frame)` 时，已入队帧之外的 token 无人认领，且 `advance` 会把
`TxQueueFull` 升级为 `fail()`，设备进入 `Failed` 且无重建路径。

### 2. `drive_shutdown` / `finish_cancel` 的在飞写处理不完整

**Flagged by**: @principal-2、@quality-1、@quality-2
**Location**: `drivers/net/aic8800/src/device/progress.rs:231-245`（`drive_shutdown`）、`:247-260`（`finish_cancel`）

`drive_shutdown` 不释放 `active_tx`（含 `extra_tokens`）与新增的 `pending_completions`，释放范围小于 `fail()`；
`finish_cancel` 不清 `active_tx`，取消后下一次 `consume_transmit_flow` 会重发作废写。
（说明：既有材料中「`finish_cancel` 丢失在途 token」一说经 @quality-1 复核后不成立，token 会被重试并归还。）

### 3. 两处 `events.push_back` 绕过事件队列的容量与淘汰策略

**Flagged by**: @principal-1、@quality-1
**Location**: `drivers/net/aic8800/src/device/progress.rs:544-547`、`drivers/net/aic8800/src/device/data_plane.rs:594-597`

直接入队绕过 `push_event` 的 `RX_CAPACITY` 边界、淘汰策略与字节记账，可让 `events` 无界增长，
并使 `event_room()` 饱和到 0，把完成全部压进 `pending_completions`——与 `b677b55b1` 的意图相矛盾。

### 4. 新增的两个推迟队列未纳入活性判定

**Flagged by**: @quality-1、@principal-2
**Location**: `drivers/net/aic8800/src/rdif/owner/output.rs:192-204`

`pending_completions`（core 侧）与 `pending_tx_tokens`（适配层）都不在 `has_pending` /
`has_runnable_pending` 中，当前靠 `queue_progress` 副作用兜住；一旦副作用不成立，owner 可在队列非空时停止推进。

### 5. `b677b55b1` 缺少红-绿证据，其两条主张无测试钉住

**Flagged by**: @testing-1
**Location**: `drivers/net/aic8800/src/device/data_plane.rs:1351`（新单测）、`rdif/owner/output.rs`

该提交改写了它要修的断言；推迟完成与适配层重试两处行为无测试覆盖（多种变异实现仍全绿）。
按项目 `test-quality` 要求，应先在 `89ff23e4f` 上写出红测试（满 `RX_CAPACITY` 时保留接收帧、
推迟完成的活性、`ring_size: 2` 下的适配层重试），确认失败后再在 `b677b55b1` 上转绿。

### 6. 聚合缺少字节上限，信用单位与厂商不一致

**Flagged by**: @principal-1、@quality-1
**Location**: `drivers/net/aic8800/src/device/owner.rs`（`set_tx_aggregation` 无上界）、
`drivers/net/aic8800/src/device/data_plane.rs::aggregate_limit`

K=4 且 MTU=1500 时一笔写恰好等于厂商 BSP 的 `MAX_AGGR_TXPKT_LEN = 1536*4`，没有余量；
而厂商命令路径的信用检查是**按字节**的（`aicwf_sdio.c:1838`：`len > buffer_cnt * BUFFER_SIZE`），
本驱动按「一个包一个缓冲」扣减。该差异是否会在读数接近保留位时透支固件池，需固件口径或对照实验判定。

## Suggestions

### 文档更正（必须同步修订 `www/sg2002/aic8800/probev5-aggregation-review-20260925.md`）

- 「`tx_ready` 深度 0 ⇒ 帧没有到达驱动」**不成立**：该深度在 `executor/mod.rs:745` 的 `finish_idle` 中、
  提交循环已排空 `tx_ready`（`:586`）之后采样，健康窗口同样近乎全 0（P4 `8787/16/1/0`、
  本轮 `4438/22/1/0`）——原文第 2.2 节第 1 点与第 6 节的中心推理作废。— @quality-2
- 「275 µs/包、快 2.8 倍」的分母是 `ActiveTx::packets()`；若每笔只投递第 1 帧，真实投递成本
  1102 µs/帧，反劣于 K=1 的 706–780 µs。— @principal-1
- 「完成事件挤占缺陷未触发」的论证用交付量（`rx_done`）代替产出量（`frames`），且淘汰路径静默无计数，
  无法从日志反证未触发。— @quality-2
- 256 KB 是发送侧应用写入量，不是「送达量」。— @quality-2

### 测试与观测

- 最省判别手段（二选一）：PC 侧统计实际收到的段/字节（`netstat -s` 或抓包）与驱动写出帧数对比；
  或把 `TX_AGGREGATION_PACKETS`（`rdif/owner/progress.rs:30`）设为 1 重跑同一用例，速率应回到 ~8 Mbit/s 量级。
  — @quality-2
- 板→PC 走 UDP 单向（`iperf3 -u -b 20M`）：无窗口、无重传，若板端持续写入而 PC 只收到约 1/4 字节即定案。 — @principal-1
- 为新增的推迟/批量路径补宿主机测试；`probe.abandon()` 目前无痕，使「日志无异常 + 计数平衡」
  无法排除取消类异常。— @testing-1、@quality-2

## What's Working Well

- 单飞行写（`active_tx` 单槽）与 `#2299` 的接收优先次序未被破坏（多位审查者读码确认）。— @principal-1、@quality-1
- 用户路径的 token 与信用「恰好一次」：完成、失败两条路径逐条核对成立（`extend_active_write` 循环有界、
  `packets == 0` 守卫正确、`submit_one_tx` 不丢环上帧且不无限循环、`consume_transmit_data` 恰好扣
  `packets()` 个信用、`promote_pending_completions` 不重复发布）。— @quality-1
- 适配层三态发布（`TxPublish`）与 `pending_tx_tokens` 重试、`flush()` 顺序正确，不会搁浅缓冲。— @quality-1
- 新增单测并非空测：多种变异实现（M1/M6/M7）能让它失败。— @testing-1

## Clarifying Questions

1. **`iperf3` 在 `-R` 模式下的统计口径**：`Transfer 256 KBytes` 是应用写入量还是 TCP 实际发出量？
   `Retr` 取自哪一端？日志里 `Cwnd 0.00 Bytes` 表明设备侧 TCP_INFO 非真值，需外部工具口径确认。— @quality-2
2. **固件对整笔 512 补齐的容错边界**：是否存在某类尺寸（`raw_len` 为 512 倍数）或某种写长度下固件能继续
   遍历后续帧？这决定聚合在哪些尺寸上「偶发可用」。— @principal-1
3. **信用单位**：D80 数据方向的 `buffer_cnt` 是「包缓冲个数」还是「1536 字节单位」？
   厂商命令路径按字节检查，本驱动按包扣减，二者需要固件口径确认。— @principal-1
4. **PC 侧接收计数**：客户端实际收到了多少字节/段？这是闭合「丢帧模型」因果链最直接的一条数据。— @quality-2、@principal-1

## Requirements Assessment

| 请求项（见 `requirements.md`） | 结果 |
| --- | --- |
| 独立核验既有事实 | **完成**，并更正式中三处推理（`ready` 采样点、275 µs/包分母、「无重传」推断） |
| 找出可能导致停住或退化的其它缺陷 | **完成**：另确认 2 个阻塞项与 4 个应修项（M2/M4 泄漏、批次中途失败、事件队列旁路、活性判定、测试缺口、字节上限） |
| 判断「固件丢弃多帧写部分帧」是否可从代码到达 | **完成**：该机制在代码层面确定成立（三条独立证据链），并给出唯一例外与修复方向；但它是否解释板端读数未能从现有日志闭合 |
| 明确列出无法判定的部分 | **完成**：计数归属（189 vs 283）、`iperf3` 口径、信用单位、以及「是否为停住的唯一原因」 |

置信度：代码层面缺陷 **高**；板端因果链 **中**（机制成立且时间上同步，但缺对端计数与可信的重传证据）。
