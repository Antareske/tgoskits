# NAPI 重构（dev 59fb32de6）对 sg2002/wifi-irq 分支的变基影响调查

日期：2026-08-27
调查对象：dev 分支提交 `59fb32de6 refactor(ax-net): add queue-level NAPI runtime (#2178)`（2026-08-26 合入）
分支基线：merge-base `8e39cbd586`，本分支 10 个提交（`1942849e6..3d9313301`）+ 8 个未提交 WIP 文件

## 1. 结论摘要

1. **冲突面很小但性质重**。全部 35 个 dev 新提交与本分支 10 个提交的文件级交集只有 4 个文件：aic8800 `fdrv/thread/rx.rs`、`os/arceos/modules/axruntime/src/wifi_glue.rs`、sdhci-cv1800 `src/irq.rs`、`src/lib.rs`。其中前两个是"API 删除型"冲突，后两个是"模型重写型"冲突（分支核心工作所在）。
2. **WIP（未提交）的冲突面更大**：8 个 WIP 文件里有 7 个被 dev 大改（`pollset.rs` 被整文件删除），约 30 处 pollset wake、`spawn_poll_task`/`block_until`/`unmask_card_irq` 调用点全部编译失败。
3. **板卡芯片确认是 D80 系（V3）**，不是 DC/DW。dev 新代码对 DC/DW fail-closed（`validate_queue_irq_variant`），不影响本板。但分支 kicker 的存在前提（ISR 不可靠）与新模型的理论前提（IRQ 以 disabled 注册、worker 固定后再 enable）相矛盾，需要板测裁决。
4. **分支的 sdhci-cv1800 XFER_COMPLETE 中断驱动 PIO 完成机制无法直接平移**：dev 新模型把硬 IRQ 端点收为 move-only（只处理 CARD_INT），`SdioIrqStatus` 无 XFER 变体，分支的 `register_pio_wake_callback` 等依赖的全局静态模型被整体删除。这是本次适配的核心设计决策点。
5. 变基尝试已中止，工作区还原到 `3d9313301` + WIP，备份分支 `backup/sg2002-wifi-irq-pre-rebase-20260827` 保留。

## 2. 59fb32de6 改动面总览

100 文件，+8855/-7639。核心是新增 queue-level NAPI 运行时子系统并迁移全部网络驱动：

| 区域 | 文件 | 规模 | 内容 |
|---|---|---|---|
| 新运行时核心 | `net/ax-net/src/queue_runtime/{mod,executor,state,spsc}.rs` + `poll_runtime.rs` | 新增 ~1850 行 | 固定 CPU 执行器、poll group 状态机、SPSC ring、协议 poll 运行时 |
| 设计文档 | `docs/docs/architecture/net/queue-napi-runtime.md` | 新增 621 行 | 新运行时权威设计文档 |
| 驱动契约 | `drivers/interface/rdif-eth/src/lib.rs` | 622 | `Interface` → `NetDevice::into_parts`、move-only `DmaBuffer` |
| 设备封装 | `drivers/net/rd-net/src/lib.rs` | 1021 | `prepare_device`、队列封装、DMA pool |
| aic8800 | 全 crate | rx 270 / tx 131 / bus 132 / device 652 / wireless 196 / ap 103 / cmd 138 / pollset **删除** 108 | 线程模型整体移除 |
| sdhci-cv1800 | `src/irq.rs` 131、`src/lib.rs` 31 | | 全局静态 → move-only IRQ source |
| sdio-host | `src/lib.rs` 33 | | `SdioIrqSource`/`SdioIrqStatus`/`take_irq_source`/`rearm_and_check_card_irq` |
| axruntime | `irq.rs` 157、`devices.rs` 174、`lib.rs` 15、`wifi_glue.rs` 33 | | `PinnedNetIrqRegistrar`、`init_net` 后移到 `online_smp` 之后 |
| ax-driver | `net/binding.rs` 71、`net/aic8800.rs` 123 | | 注册路径改 `register_net_with_info` |

## 3. 新运行时契约要点（驱动侧视角）

- **所有权**：驱动在 `into_parts()` 一次性交出队列（`ITxQueue`/`IRxQueue`）、IRQ 端点（`NetHardIrqEndpoint`）、IRQ 控制（`NetPollIrqControl`）、可选 owner 启动（`NetOwnerStartup`）与 WiFi 控制（`WifiControl::execute`）。
- **执行模型**：每 CPU 一个 `net-queue-cpu{N}` 固定亲和 worker；IRQ 由运行时经 `PinnedNetIrqRegistrar` 注册（Shared + NonReentrant + `Fixed(owner_cpu)` + auto_enable=No），驱动不接触 IRQ 注册表。硬 IRQ handler 只做有界工作（读状态/mask/返回 `Schedule(snapshot)`），禁止持锁、唤醒、分配。
- **收尾协议**：poll 收尾时任务侧调 `rearm_and_check()` 原子闭合 drain→rearm 窗口（`SdioCardIrq::unmask_card_irq` 已删除）。空闲 worker 零周期唤醒，预算 64 项/阶段/轮。
- **DMA token**：`DmaBuffer` move-only，`submit` 失败必须经 `SubmitError` 归还；`reclaim` 返回 token 本体而非 bus_addr；错误路径 token 保留（`NetError::Retry` 语义收紧）。
- **控制面**：命令等待从 waker 注册改为 deadline + `progress_io()` + `yield_now()` 协作循环；AP 对账从 50ms 周期定时器改为事件驱动。
- **启动时序**：probe 只识别芯片 + `take_irq_source()`；固件/FDRV 加载移到 `NetOwnerStartup::initialize()`（worker 固定后执行）；`init_net` 移到 `fs::online_smp()` 之后。

## 4. 分支侧冲突映射

### 4.1 已提交部分（4 个文件）

| 文件 | 分支侧 | dev 侧 | 冲突性质 |
|---|---|---|---|
| aic8800 `fdrv/thread/rx.rs` | `1942849e6` 将 10ms kicker 从 DC/DW 专属改为全变体启用（ISR 路径不可靠兜底） | 整个 `start()`/`start_rx_poll_kicker()`/RX 数据回调删除，改为 `pub fn process_rx_frames(bus, budget) -> usize` | API 删除型。kicker 动机与新模型前提矛盾（见 §6.1） |
| axruntime `wifi_glue.rs` | sdhci PIO 系列提交加 42 行：`SDHCI_PIO_WQ` + `block_timeout_until` + `sdhci_pio_wake_callback` + `register_pio_wake_callback` + 单核 debug_assert | 删除 `spawn_poll_task`/`block_until` impl（33 行纯删除） | 两处改动区域相邻但不重叠，可机械合并；但分支代码依赖被删的 sdhci irq.rs 全局 API（见下） |
| sdhci-cv1800 `src/irq.rs` | 8 个提交的全局静态模型：`SdhciIrqState` + `register_card_irq_callback` + `register_pio_wake_callback` + ISR 处理 CARD_INT 与 XFER_COMPLETE（mask + 唤醒回调，sticky 位留给任务 W1C）+ `unmask_xfer_complete_signal` | 整体重写为 move-only `CviSdhciIrqSource: SdioIrqSource`（只处理 CARD_INT，返回 Spurious/CardPending）+ `rearm_card_irq_and_check` + `enable_irq_signals(base)`，删除全部全局状态与回调 | **模型重写型，本次适配最大冲突**。XFER_COMPLETE 唤醒路径在 dev 模型中不存在（见 §6.2） |
| sdhci-cv1800 `src/lib.rs` | PIO 等待机制（Phase 1 轮询 / Phase 2 `block_timeout_until`、W1C 语义、drain store buffer 等约 490 行） | 两处：`enable_interrupts_irq`（信号保持屏蔽，由运行时使能）+ `SdioHost` impl（`enable_irq`/`disable_irq` 带 base、新增 `take_irq_source`） | 分支的 Phase 1/Phase 2 主体（hunk `-108` 与 `-243`）无重叠可干净应用；`enable_interrupts_irq` 与 `SdioHost` impl 两处 hunk 冲突 |

### 4.2 WIP（未提交，8 文件）

| 文件 | WIP 内容 | dev 侧 | 后果 |
|---|---|---|---|
| `core/pollset.rs` | Mutex → `raw_lock::RawSpinLock` | **整文件删除** | WIP 修改作废，无迁移价值 |
| `core/bus.rs` | `raw_lock` 替换 + `fc_blocked_count` 新字段 | import 重写、TxState 删 2 pollset 加 `completed`、`shutdown` 重写 | 三方冲突最重，需在新结构上重新应用 WIP 意图 |
| `core/mod.rs` | `pub mod raw_lock;` | 删 `pub mod pollset;` | 同区平凡冲突 |
| `core/sdio_transport.rs` | raw_lock 替换等 | `unmask_card_irq` → `rearm_and_check_card_irq`、`card_irq` 非 Option、新增 `clear_v3_other_interrupt` | 冲突 + 调用点签名迁移 |
| `thread/rx.rs`、`thread/tx.rs` | kicker/唤醒双检查/信用门控等（tx.rs +281 行） | 线程模型整体删除，改 budget 推进 + DMA token | 约 30 处 pollset wake 编译失败；tx WIP 的流控/信用门控需按新模型重表达 |
| `Cargo.toml` | 自加 dev-dependencies ax-sync | 上游也加 dev-dependencies ax-sync（path 写法）并删 `atomic-waker`、`sdhci-cv1800` 依赖 | dev-deps 合并；删除的依赖在变基后无残留引用（待确认） |
| `core/raw_lock.rs`（新增） | RawSpinLock 实现 | 无 | 文件存活，但可能成为死代码，需在新模型下重新评估锁需求 |

## 5. 芯片变体确认（关键前提）

- 板卡日志 `[fdrv] flow_ctrl OK, reg=0x03, val=127`：0x03 是 `SDIOWIFI_FLOW_CTRL_Q1_REG_V3`，只有 `is_v3()`（= `Aic8800D80 | Aic8800D80X2`）走该寄存器 → **板卡芯片是 D80 系 V3**。
- dev 新 `validate_queue_irq_variant` 允许 `Aic8801 | Aic8800D80 | Aic8800D80X2` → 本板卡通过校验，dev 对 DC/DW 的 fail-closed 不影响本板。
- 分支内大量 DC/DW 双管道适配（`cmd_func()==2` 等）在本板不生效；dev 新版 rx 改为只排空 func1（与厂商 ISR 变体语义一致），与本板行为兼容。

## 6. 适配的关键设计决策点

### 6.1 kicker 的去留（aic8800 rx/tx）

- 分支前提：probe 阶段 PLIC IRQ 未使能，D80/8801 ISR 路径不可靠 → 10ms kicker 兜底。
- 新模型前提：IRQ 以 disabled 注册，worker 固定 CPU 且全部就绪后才 `enable()`；`rearm_and_check` 原子闭合重挂窗口。PLIC 未使能的前提理论上被消除，且新运行时契约没有周期任务机制。
- 结论：**先按 dev 无 kicker 版本板测**；若真实硬件仍有丢帧，需要扩展运行时（周期唤醒不在现有契约内），这是行为验证点而非纯代码问题。

### 6.2 XFER_COMPLETE 中断驱动 PIO 完成的移植（sdhci-cv1800，分支核心价值）

- 分支机制：PIO Phase 2 阻塞在 `block_timeout_until`，ISR 见 XFER_COMPLETE 即 mask 信号 + 唤醒 WQ，sticky 位由任务 W1C 消费；配合 selective W1C、错误路径消费、lost-wakeup 窗口闭合等修复，上传吞吐 0.85 → 11 Mbps。
- dev 新模型：硬 IRQ 端点 move-only 且只处理 CARD_INT；`SdioIrqStatus` 只有 `Spurious | CardPending`；无 PIO 唤醒回调概念。
- 候选方向：
  a) 扩展 `SdioIrqStatus`/`CviSdhciIrqSource::handle_irq` 增加 XFER 分支（需要改 sdio-host 契约 + 运行时 handler 桥接，波及面大）；
  b) XFER_COMPLETE 唤醒不经过网络运行时端点：`CviSdhci` 的 PIO 等待与网络队列运行时的 CARD_INT 端点共享同一物理 IRQ，需评估 Shared IRQ 注册下第二个 handler 的可行性（新模型 IRQ 注册为 Shared + 固定亲和，但 handler 形态是 `PinnedNetIrqAction`，PIO 唤醒要另立入口）；
  c) PIO 完成退回纯轮询（放弃分支的中断驱动 PIO，性能回退到 0.85 Mbps 基线，不可接受）。
- 这是本次适配的核心设计决策，涉及 sdio-host 契约、axruntime IRQ 注册与 sdhci 驱动三层的改动，建议单独成文设计后再动手。

### 6.3 分支单核假设 vs 新运行时 SMP 模型

- 分支 `wifi_glue.rs` 有 `debug_assert!(cpu_num() == 1)`（release 编译掉）；分支 sdhci irq.rs 注释明确单核假设。
- 新运行时按 `online_cpus` 分配 owner CPU，SMP 就绪。板卡实际单核，但代码层面的假设需要在新模型下重新表述或保留为文档化约束。

### 6.4 raw_lock 的定位

- WIP 引入 `raw_lock::RawSpinLock`（Mutex 在 irqsave 下 might_sleep panic 被板测排除，raw 为唯一候选，见锁回归记录）。
- 新模型的锁面完全不同：IRQ 路径零锁（handler 只做有界工作），驱动回调在 owner CPU 任务上下文单线程串行，剩余的锁（WifiControlQueue、completion 等）用 SpinLock 即可。
- 结论：raw_lock 大概率不再需要；变基后先按 dev 的锁模型落地，若板测再现 might_sleep panic 再针对性引入。

## 7. 适配工作块评估

| 工作块 | 规模 | 内容 |
|---|---|---|
| 变基执行 | 小 | 4 个文件冲突按 §4.1 处理；WIP 先搁置（`git stash`），提交级变基完成后在 dev 新模型上重新开发 |
| sdhci XFER_COMPLETE 移植 | 中大 | §6.2 设计先行；分支 490 行 PIO 等待主体可保留，ISR 侧重写 |
| aic8800 变基 | 小 | 分支只有 rx.rs 1 处提交级改动；dev 的 device.rs 等重写直接套用 |
| WIP 迁移 | 中大 | tx.rs +281 行流控/信用门控、rx 双检查逻辑在新模型下重表达；raw_lock/fc_blocked_count 重新评估 |
| 板测验证 | 中 | kicker 去留裁决、XFER_COMPLETE 移植后上传吞吐回归（基线 11 Mbps）、SoftAP/STA 功能矩阵 |

## 8. 回退与状态

- 分支状态：`sg2002/wifi-irq` @ `3d9313301`，WIP 8 文件完整（变基已 `--abort`，stash 已恢复）。
- 备份：`backup/sg2002-wifi-irq-pre-rebase-20260827`（= `3d9313301`）。
- 参考：旧模型与新模型的完整契约对照在 `.sg2002-build/sources/dev-wififix`（dev 检出于此）可直接查看 59fb32de6 版本代码；新运行时设计文档 `docs/docs/architecture/net/queue-napi-runtime.md`。

## 9. 变基执行记录（2026-08-27 完成）

执行：WIP 快照 commit（`36b89dd62`）→ 备份 `sg2002/wifi-irq-beforeNAPI`（含 WIP）→ 本分支回退 `3d9313301` → `rebase dev` 重放 10 个提交。变基结果：dev `ba252ca67` + 10 个提交（`70e3ed000`..`5b4e64d3a`），对 dev 净 diff 6 文件 +470/-80。

各提交的适配：

| 提交 | 适配 |
|---|---|
| `70e3ed000` wifi boot | rx.rs kicker hunk 丢弃（新模型无 `spawn_poll_task`），仅保留 its/toml 配置 |
| `cd3e76418` 中断驱动 PIO | irq.rs 在新 move-only source 模型上重表达：`register_pio_wake_callback` 全局槽位（单控制器假设）、ISR 对 XFER_COMPLETE 只 mask 信号 + notify、sticky 位留给任务 W1C；`unmask_xfer_complete_signal(base)` 参数化；`enable_interrupts_irq` 保持 dev 全屏蔽语义；wifi_glue import 合并 |
| `26f6a94ad`/`a09d98483`/`f55f02841` | lib.rs Phase 1/Phase 2 区域无重叠，干净应用 |
| `2a24a8aac` 注释中文化 | 冲突区取新模型侧（新注释已为中文），残余翻译正常应用 |
| `5b4e64d3a` 丢唤醒窗口 | `block_timeout_until` 条件等待协议保留，仅适配 `unmask_xfer_complete_signal(self.base)`；协议要点补入 irq.rs 模块文档 |

检查：`cargo fmt` 无改动；clippy 通过 `sdhci-cv1800`、`ax-runtime`（22 项含 net 特性）。

**待板测裁决**：kicker 是否需要（新模型前提理论消除 ISR 不可靠）、上传吞吐回归（基线 11 Mbps）、SoftAP/STA 矩阵、Phase 2 退化与错误误报计数为零。WIP（raw_lock + TX 信用门控）只存在于 beforeNAPI，需在新模型上重新评估后再决定是否开发。
