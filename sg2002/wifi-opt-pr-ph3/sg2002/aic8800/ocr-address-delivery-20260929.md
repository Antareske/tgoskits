# AIC8800 OCR 修复阶段交付记录

日期：2026-09-29  
工作树与分支：`wt-sg2002-wifi-opt-pr-ph3` / `sg2002/wifi-opt-pr-ph3`  
审查来源：`.ocr/sessions/2026-09-29-sg2002-wifi-opt-pr-ph3/rounds/round-1/final.md`  
状态：**修复已落地、已提交并通过 host 侧验证**（`cargo xtask test` 全绿、两组 clippy 全绿、fmt 干净）。提交为 `2f3f97baf`（代码与测试）与 `4282949bc`（正式文档）；未推送，实板未验证。板测镜像见 `board-image-ph3-20260929.md`。

## 1. 目的与范围

本记录承接 OCR 的 `REQUEST CHANGES`，说明获准修复方案、实际落地的代码、验证证据、未完成项和外部证据限制。修复范围限于 `sg2002/wifi-opt-pr-ph3` 上 AIC8800 前三阶段优化 PR 的相关内容：恢复误删的上游诊断与设计契约，统一 TX 聚合和信用等待规则，检查 SDIO 长度边界，保障聚合 token 所有权，并对齐相应测试与正式设计文档。

本轮没有切换或创建分支，没有创建/修改提交，也没有推送。工作树中已有的 `AGENTS.md`、`CLAUDE.md`、`.claude/`、`.ocr/` 属于本地状态，不纳入追踪。本文位于个人 `www/`，不属于项目追踪内容。

## 2. 原定方案

### 2.1 恢复项目范围与正式文档

- 从 `origin/dev` 恢复 mailbox 超时日志中的 `startup_stage=` 信息，以及启动 confirmation 失败诊断函数和调用点。这些上游诊断与本次优化无关，不应被清理。
- 在 `docs/design/unified-sdio-aic8800.md` 中恢复仍由当前代码实现的两条协议契约：运行期 mailbox credit backoff 可等待中断并继续 IRQ 驱动 RX scan；启动期 backoff 返回 `RetryAt`。启动期间 CARD_INT 保持屏蔽，直到 mailbox 写入并进入 confirmation wait 后才允许卡中断驱动接收扫描。保留已有 FriendlyARM vendor-tree 来源描述。
- 修正新增设计文字：续接发生于 `IoPurpose::TransmitData` 完成分支；单包写发出 `TransmitComplete`，多包写通常发出 `TransmitAggregateComplete`；事件队列已满时 deferred token 后续逐 token 发布，不保证仍保持聚合事件形态。

### 2.2 统一聚合参数契约

采用 `TxAggregation::bytes` 的现有实现语义：这是每笔 TX stream 的软目标，检查发生在追加帧之前，因此允许最多一帧越过目标；它不是 TX DMA ring 容量硬上限。

- core setter 和 `AicRdifDevice::new` 对零 packet/byte limit 明确报错，不静默 `.max(1)` 修复。
- FDT 对显式聚合属性验证非零；未设置属性时保留 adapter 默认值，即使 queue/frame 容量较小也不应因默认值超出 ring bytes 而失败。
- 移除 `queue_size * frame_size` 作为 bytes 硬上限；同步更新板级配置文档和软边界测试描述。

### 2.3 信用、长度与 token 所有权

- credit 不高于命令保留量时保持未缓存状态。
- 薄池等待只用于仍有排队用户帧、聚合策略允许多包且当前写仍能追加的情况；其 credit 阈值受 policy 限制、但最多按现有上限等待，预算耗尽后必须写出可用 credit。
- Ethernet TX 编码拒绝超过 12-bit SDIO header 可表示长度的帧。stream 长度在编码时校验并随已验证帧返回；聚合器不再从线路字节重新掩码解析，也不以整块 padded 长度作回退。
- 用聚合 burst 覆盖 shutdown、cancel、fail 下的 token 恰好一次回收。
- aggregate completion 中遇到未知 token 时继续处理该批次其余已知 token，避免把有效 DMA buffer 留在 `tx_tokens` 中长期占用。
- RDIF 测试应经过真实 queue、`take_tx_batch`、completion 和 flush 行为，不通过直接写 `pending_tx_tokens` 来证明 production path。

### 2.4 范围与证据

- 保留单帧 `AicInputEvent::Tx` 公共 enum 变体以避免额外 API 删除；补文档并让其满队策略与 batch 一致。
- `0x140b` 保持 credit update indication 分类；`www/sg2002/iperf3-tests/linux/sta/board_serial.log` 中厂商驱动在同一块板上的日志为 `rwnx_rx_handle_msg msg->id:0x140b` 紧跟 `rwnx_rx_me_tx_credits_update_ind()`，即该 id 是 credits update 而非 traffic indication。`0x140d` 没有 AIC 专属、可追溯的仓库 vendor source，按“只作为未被请求的异步指示被忽略”保留，并在注释中点明取值来自厂商 LMAC 消息表。不得在无适用证据时修改 capability payload bytes。
- 清理已删除 telemetry 的注释、`p0a_reference` 私有阶段名，以及已核实不准确的边界/注释。
- 不改写提交标题，不提交、不推送。

## 3. 已实现

### 3.1 上游诊断与正式文档

- `device/startup/mod.rs` 恢复 `startup_stage_diagnostic()` 与 `log_startup_confirmation_error()`；`device/mailbox.rs` 的超时日志重新包含 `startup_stage=`，并在启动 confirmation 失败时记录阶段、响应长度与响应头。启动路径才复制响应头字节，控制路径不再为此分配；日志记录的是映射后的错误，因此包含请求身份。
- `docs/design/unified-sdio-aic8800.md` 恢复运行期/启动期 mailbox backoff 的区分、CARD_INT 启动时序两条契约，以及 FriendlyARM vendor tree `174d4e6989914651850b3ba52c7880a458aa3602` 的来源引用。
- 同一文档修正新增文字：帧流布局由 `ethernet_tx_frame()` 返回的流内长度维护、不再从已编码字节重新解析长度；`TxAggregation::bytes` 是追加前检查的软目标、不是环容量硬上限；单帧写发 `TransmitComplete`、多帧写发一个 `TransmitAggregateComplete`；完成环满时保留待完成 token，事件队列满时 token 先暂存并随后逐个发布；续接发生在 `IoPurpose::TransmitData` 完成分支；薄池等待只作用于仍能增长的用户写；FDT 只覆盖显式属性并拒绝显式零值。
- 文档「测试边界」新增一段：帧流遍历与“每个写入 packet 消耗一个固件 buffer”两项固件假设无法由 host 测试证明。

### 3.2 聚合参数契约

- `TxAggregation::is_valid()` 成为零值判定的唯一来源；`AicDevice::set_tx_aggregation()` 返回 `Result`，零值返回新增的 `AicError::InvalidTxAggregation`，不再 `.max(1)` 修复。
- `AicRdifDevice::new()` 校验 options 中的聚合策略，新增 `AicRdifError::InvalidTxAggregation` 并映射为 `NetError::InvalidParts`（不是可重试的队列耗尽）。
- `fdt.rs` 只在属性显式存在时覆盖 adapter 默认值，随后做一次非零校验；删除 `queue_size * frame_size` 硬上限比较，未写属性的板级配置不再因默认值超出小环而 probe 失败。
- `MeConfigProfile` 的选择上提为 `ChipProfile` 的字段，与芯片支持表同源：`MeConfigProfile::for_chip` 删除，启动 FSM 中不再存在 `UnsupportedChip` 分支，两张表无法再漂移。

### 3.3 信用、长度与 token 所有权

- reserve 读数不写入缓存：`tx_credits` 只在读数高于命令保留量时保存。
- 薄池等待条件收敛为“用户写 + 策略允许多帧 + 仍有排队帧 + 当前写未越字节目标”；阈值按 policy 的帧数上限计算，并受 `DATA_TX_MAX_WAIT_CREDITS` 上限约束；预算耗尽后照发可用 credit。生命周期帧与单帧策略直接用可用 credit。
- `ethernet_tx_frame()` 返回 `(frame, stream_len)`，并在声明长度放不进 12-bit 字段时失败；`stream_frame_len()` 及其测试删除，长度不再有第二个解析器。`ActiveTx::new()` 与 `append_frame()` 只接收已验证的流内长度，追加路径不再解析帧头。
- `TxState::take_wire_frame()` 返回命名的 `WireFrame { token, bytes, stream_len }`，`Err` 分支携带需要归还的 token。
- 停机时先归还活动写与排队 token（`drive_shutdown()` 在发出 shutdown 写之前交付这些完成事件），再发 SDIO shutdown。
- aggregate completion 遇到未知 token 时继续处理批次内其余 token，批次结束后报告 `CompletionMismatch`；未被点名的 packet 保持其 buffer 所有权。
- 单帧 `AicInputEvent::Tx` 补上文档，满队时与 `TxBatch` 同策略：归还 token、设备保持 Ready，不再 `TxQueueFull` 失败整机。

### 3.4 测试与 fixture

- 新增 `drivers/net/aic8800/src/rdif_test_support.rs`（库内 `#[cfg(all(test, feature = "rdif"))]` 模块）：提供身份映射的 host DMA 分配器与 `dma_buffer(len)`，使 owner 测试能构造真实 move-only `DmaBuffer`。测试依赖按 `rd-net`/`rdif-eth` 的既有形态声明：dev-dependency `dma-api = { features = ["pool"] }` 加 `cfg(not(target_os = "none"))` 下的 `ax-runtime` host-test，模块内 `extern crate ax_runtime as _;` 链接 spin provider。
- 新增/改写测试：
  - `a_single_frame_that_does_not_fit_is_reported_complete`（红-绿已确认：旧实现下设备进入 `Failed`）
  - `shutdown_reclaims_a_queued_aggregate_continuation_before_sdio_shutdown`（红-绿已确认：旧实现先发 shutdown 写）
  - `tx_aggregation_rejects_a_zero_limit_instead_of_repairing_it`
  - `a_pool_that_cannot_back_the_batch_does_not_buy_a_smaller_write`
  - `reserve_credit_read_is_not_cached`、`thin_credit_wait_is_bounded_and_eventually_writes_the_available_batch`
  - `full_return_ring_holds_the_overflow_until_the_ring_drains`、`aggregate_mismatch_returns_every_buffer_the_completion_named`、`bounded_batch_take_moves_real_buffers_and_limits_the_producer_batch`（均经真实 SPSC 环与真实 DMA buffer）
  - `ethernet_tx_rejects_lengths_that_do_not_fit_the_sdio_header`、`the_soft_byte_target_allows_one_crossing_frame_then_stops_growth`

### 3.5 注释与残留清理

- `p0a_reference` 改名 `conservative_reference`（私有阶段名不外泄）。
- `phy_bw_max` 的两个相同 match 臂收敛为一个具名常量 `ME_CONFIG_PHY_BW_MAX`；VHT 最高速率注释不再宣称带宽，与“不宣告 80 MHz”的能力位不再冲突。
- `0x8000_0001` 命名为 `FIRMWARE_CONFIRMATION_HOST_ID`；HT MCS 字段长度 `16` 命名为 `HT_MCS_LEN`；两处只复述零初始化的 `fill(0)` 删除。
- 清理 telemetry 残留措辞与「two thirds of an acknowledgement stream」比例、A-MSDU 的“证据/计数”表述；`complete_write_tokens`、`padded_wire_write`、`append_frame`、`TxPublish` 三个变体、`aggregation_credit_threshold` 等注释改为与代码一致。
- `take_active_write_tokens` 上方孤立的谓词注释归位到 `user_write_in_flight`。

## 4. 未实现或未闭合

### 4.1 实板与固件假设

- 帧流遍历规则与“每个写入 packet 消耗一个固件 buffer”的计数规则来自厂商固件约定，host 测试只能证明驱动自身的编码与计数自洽；真实 SDIO 中止后的 DMA 回收也只有实板能证明。三者已写入设计文档的测试边界，若形成 PR 需按 affected-but-unverified 列出。
- 未在实体 LicheeRV Nano 上运行；本轮没有镜像构建与烧写。

### 4.2 未加的测试及原因

- FDT 聚合属性解析没有单测：`rdrive::probe::fdt::FdtInfo` 只能由真实设备树经 probe 路径构造，而 `net/aic8800` 模块位于 `aic8800-wifi` feature 之后，当前没有任何 host-test profile 打开该组合；新增 profile 会让整个 ax-driver 测试套件在新 feature 组合下重跑。零值判定本身已在 core（`AicDevice::set_tx_aggregation`）被测试，FDT 侧复用同一 `is_valid()`，没有第二份判定逻辑。
- `AicRdifDevice::new()` 的 options 校验没有单测：构造它需要一个完整的 `CompletionIrqRearmHost` mock，成本与收益不成比例；该边界只是把同一判定前移。

### 4.3 明确不做

- 第一个 commit subject（`perf(aic8800): retain direct data-plane optimizations`）未改写：项目规则禁止在未获授权时改写历史；OCR 建议的 `perf(aic8800): aggregate transmit writes and cache firmware credits` 留给提交阶段决定。
- `AicInputEvent::Tx` 的另一种处置（删除该变体、测试改走 `TxBatch`）未采纳，保留公共 API 以免额外破坏。

## 5. 验证记录

| 命令 | 结果 | 说明 |
|---|---|---|
| `cargo xtask test` | 通过 | 71/71 软件包全部通过，含 `aic8800` 的 `host-test+rdif` profile（lib 160 + 集成 1 例）；结束状态 `all std tests passed`。 |
| `cargo xtask clippy --package aic8800` | 通过 | 3/3 检查（base、`rdif`、`host-test`）。 |
| `cargo xtask clippy --package ax-driver` | 通过 | 54/54 检查，含 `aic8800-wifi` 组合。 |
| `cargo clippy -p aic8800 --tests --no-default-features --features host-test,rdif` | 通过 | 补充检查：项目入口的 clippy 组合不编译 `rdif` 下的 `#[cfg(test)]` 代码，故单独跑一次该 profile 的 tests 目标。 |
| `cargo fmt --all --check` | 通过 | 无待格式化差异。 |
| `git diff --check` | 通过 | 无 whitespace error。 |
| 红-绿验证 | 通过 | 单帧满队策略与停机归还顺序两项测试，均先在旧实现上失败、再在修复后通过。 |
| 实体板测试 | 未运行 | 不声称帧流、credit-per-packet 与真实 abort 回收已在板验证。 |

## 6. 工作树与收工状态

- 当前分支为 `sg2002/wifi-opt-pr-ph3`，本轮修复已提交为两笔：`2f3f97baf`（聚合上限、发送长度、token 所有权、启动诊断与测试夹具）与 `4282949bc`（正式设计文档）。未推送。
- 提交内容覆盖：核心（`device/{data_plane,mailbox,model,owner,progress,startup/mod}.rs`、`{lmac,profile,protocol,tx,lib}.rs`）、RDIF（`rdif/{device/endpoints/device,error,owner/{mod,output,progress}}.rs`）、适配层（`ax-driver/src/net/aic8800/fdt.rs`）、正式文档、`Cargo.toml`/`Cargo.lock`，以及新增的项目文件 `drivers/net/aic8800/src/rdif_test_support.rs`。
- 用户本地的 `AGENTS.md`、`CLAUDE.md`、`.claude/`、`.ocr/` 保留原样，未纳入提交。
- 板测镜像已按本轮口径出好，见 `board-image-ph3-20260929.md`。
- **下一步**：实板验证（含 shutdown/cancel 下的真实回收、聚合吞吐与 credit 等待读数）；若形成 PR，按项目规则在 PR 正文列出实板未验证项。
