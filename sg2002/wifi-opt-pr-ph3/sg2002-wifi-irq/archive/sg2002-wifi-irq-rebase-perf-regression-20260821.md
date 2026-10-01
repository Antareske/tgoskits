# SG2002 WiFi 变基性能回归：现象、根因与验证（2026-08-21）

## 概述

2026-08-21 `sg2002/wifi-irq` 分支变基到最新 dev 后，板卡实测单向 iperf3 吞吐较变基前下降约 20%，且首次双向测试出现 `data flow ctrl timeout` 告警。排查确认根因为 dev 锁统一重构（1ab948f77）改写了 aic8800 驱动的锁语义，并通过对照实验验证恢复。

## 现象

变基前基线为 `www/logs/irq-rc1.log`（2026-08-15，rc 复审配套板测），变基后为 `www/logs/rb.log`。

| 测试 | 变基前 | 变基后 | 备注 |
|---|---|---|---|
| 上行（receiver） | 11.2 / 12.0 Mbps | 9.29 / 9.61 | 变基后速率在 1.00/1.12 MB/s 间锯齿 |
| 下行（sender） | 11.0 Mbps | 8.54 | 10 个区间中 8 个恰好 8.38-8.39 Mbps（= 1.00 MB/s） |
| 双向（首次） | 5.37 / 5.81 | 3.23 / 3.34 | 变基后首秒内出现 3 次 `data flow ctrl timeout` |
| 双向（第二次） | — | 5.59 / 5.70 | 无告警，与变基前相当 |

要点：

- 双向聚合吞吐（~11.3 Mbps）与变基前持平，说明主机总吞吐能力未下降，退化的是单流场景的调度/批量节奏；下行速率被量化钉死在 1.00 MB/s 步进是节奏量化的直接证据。
- `data flow ctrl timeout` 为 `drivers/net/aic8800/src/fdrv/thread/tx.rs` 流控轮询（50 次 CMD52 + yield）窗口内 FW 信用未恢复所致，属双向启动瞬态 + 检测窗口墙钟时间变化的症状；变基前所有日志中该告警零出现。

## 排查范围

按 plic、aic8800、wifi glue 三个方向核对变基差异（旧头 af47889ce 与新头 3d9313301 逐文件对比）：

| 方向 | 变基实际改动 | 结论 |
|---|---|---|
| PLIC | 仅 `send_ipi_to_cpu` 重写（159c16bcb，IPI 门铃路径） | 排除，单核不参与数据路径 |
| aic8800 | 代码零改动（仅 components/ → drivers/net/ 移动） | 行为变化来自其锁语义被 dev 改写，见下 |
| wifi glue | 零改动 | 排除 |
| dtb | 根节点加 `dma-noncoherent`（21ef4b218）、UART1 clock-frequency（95004e621） | 排除：wifi 走 PIO 无 DMA 缓冲；UART1 不参与数据路径 |
| ax-net | TCP snoop 校验化（bed567932） | 仅 RX 侧且成本极小，不能解释双向一致的下降 |

## 根因

dev 的锁统一重构 **1ab948f77（refactor(sync): unify lock primitives in ax-sync #1956）** 将 aic8800 全部总线锁从 `ax_kspin::SpinRaw`（纯自旋，不改变执行上下文）替换为 `ax_sync::SpinLock`（`lock()` 禁用内核抢占）：

- 受影响锁：sdio 锁（每个 CMD52/CMD53）、tx 队列、rx 队列、state、rsp_queue 等全部热路径（`drivers/net/aic8800/src/fdrv/core/bus.rs`、`sdio_transport.rs`、`pollset.rs`）。
- 单核 C906 上，TX/RX 线程与 iperf3 用户态任务共享 CPU。变基前驱动临界区完全可抢占；变基后每个 SDIO 操作（含 ~50µs Phase-1 自旋）都是不可抢占窗口，且每次锁释放走新的 preempt-exit 协议（scheduler baton 状态机，8386094e6/7d4fc2723/35aaf4003）。调度交错改变后，wifi 线程的唤醒协议（register-waker→unmask 双检、10ms kicker 兜底）退化为批量节奏，单向吞吐被量化。
- fc 轮询 50 次窗口的墙钟时间随调度变化缩短，双向启动瞬态期间先于 FW 信用恢复耗尽 → 告警。

## 验证实验

在 `drivers/net/aic8800/src/fdrv/core/raw_lock.rs` 用 `SpinLock::lock_raw()`（CONTEXT_RAW，不改执行上下文）包装回 SpinRaw 语义，替换上述三处锁别名，69 个 `.lock()` 调用点零改动。`cargo xtask clippy --package aic8800`（-D warnings）与 `cargo fmt` 通过。

结果见 `www/logs/irqback.log`：

| 测试 | 变基前 | 变基后 | raw 锁实验 |
|---|---|---|---|
| 上行 | 11.2 / 12.0 | 9.29 / 9.61 | 12.3 |
| 下行 | 11.0 | 8.54 | 11.1（无量化钉死） |
| 双向 | 5.37 / 5.81 | 首测 3.23/3.34 + 3×告警 | 5.65 / 6.28，零告警 |

三项观察点全部命中，假设成立：**根因是 1ab948f77 对 aic8800 锁语义的改写，而非分支自身的 sdhci/IRQ 改动。**

## 后续选项

1. 保留 raw_lock 作为修复：把 `raw_lock.rs` 注释从实验性改写为修复依据后提交。
2. 结构性修复：驱动改为锁内不阻塞（SDIO 锁不跨 WaitQueue 等待持有），消除抢占比改变与 Phase-2 WQ 阻塞时 preempt 深度 3 触发 `blocked_resched` 断言的隐患，改动面较大。
3. 仅归档证据，暂不处理。

当前工作区状态：实验改动未提交（`raw_lock.rs` 新文件 + `bus.rs`/`sdio_transport.rs`/`pollset.rs` 各 1 行 import 替换）。

## 参考

- 日志：`www/logs/rb.log`（变基后）、`www/logs/irqback.log`（raw 锁实验）、`www/logs/irq-rc1.log`（变基前基线）
- 历史文档归档于 `www/archive/`（2026-08-21 归档）
- 提交：1ab948f77（锁语义改写）、8386094e6（scheduler frame）、0340ed6bf（定时器 catch-up，次因已随实验排除）
