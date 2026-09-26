# aic8800 异步化与流水线：调研与设计（2026-08-22）

状态：调研 + 设计草案，未实现。定位：为 aic8800 后续架构演进的决策依据。
文档内路径均为相对仓库根的路径。

---

## 1. 背景与问题陈述

### 1.1 已观测到的三个事实

1. **吞吐被定时节奏量化**：变基回归中下行 8/10 区间恰好钉在 1.00 MB/s（`www/logs/rb.log`，分析见 `www/sg2002-wifi-irq-rebase-perf-regression-20260821.md`）。1.00 MB/s 步进是 10ms 周期节奏接管流水线的指纹：数据驱动唤醒（CARD_INT → waker）一旦推迟或丢失，吞吐就掉进 10ms kicker 的周期里。
2. **流控等待是纯轮询**：TX 侧 `check_data_flow_control` 最多 50 次"读寄存器 + yield_now"（`drivers/net/aic8800/src/fdrv/thread/tx.rs:255-268`），`wait_flow_ctrl` / `wait_flow_ctrl_for_size` 是无事件关联的 yield 循环（`drivers/net/aic8800/src/fdrv/core/sdio_transport.rs:270-289`）。
3. **锁跨阻塞等待持有**：transport 锁跨整个传输含 Phase 2 XFER_COMPLETE 阻塞等待（`sdio_transport.rs:166-173`），造成调度器契约违反（`blocked_resched` 的 `can_preempt(2)` 断言隐患，`os/arceos/modules/axtask/src/run_queue.rs:966-974`），并让等待期总线状态失去保护（UP 上锁互斥溶解时，另一任务可误入总线破坏单 waiter 完成协议，详见 6.1）。

### 1.2 外部强约束（设计红线）

ax-net 在**关中断**下穿越驱动接收/发送链：

- `net/ax-net/src/device/ethernet.rs:620-624`：`Device::recv` 在 `driver.lock_irqsave()` 持锁下调用 `inner.receive()` 整条链，直达 aic8800 `AicRxQueue::reclaim`（`fdrv/net/device.rs:290-302`）→ `rx.data_queue.lock()`；
- `ethernet.rs:276`：`hardware_address()` 在 irqsave 下调 `mac_address()` → `sta_mac` 锁；
- TX 侧同理：`send_to` 在 irqsave 下走 `enqueue_data_frame` → `tx.queue` 锁（`tx.rs:548-564`）。

后果（已由 2026-08-22 板测 m1.log 实证）：这些**边界锁**被获取时中断是关的，睡眠互斥锁（`ax_sync::Mutex` 入口无条件 `might_sleep`，`os/arceos/modules/axtask/src/sync/mutex/mod.rs` `lock_plain` → `api.rs:599` `panic_atomic_sleep`）在关中断下必炸。**异步化设计必须尊重这条边界：边界锁保持同步短临界区，异步化只发生在内部线程。**

### 1.3 目标

1. 消除 1.1 中的三处轮询病灶，把唤醒从"定时兜底"改为"事件驱动"；
2. 流水线化：TX 在信用恢复的当刻续上发送，避免"等米下锅"式的空窗；
3. 以正确性为首：任何改造必须先给出丢失唤醒/竞态分析，再谈收益。

---

## 2. 现状架构地图（aic8800 线程与唤醒协议）

### 2.1 线程（全部是 `block_on(poll_fn)` 驱动的 Future 循环）

`WifiRuntime::spawn_poll_task`（`fdrv/runtime.rs:36-58`）→ `os/arceos/modules/axruntime/src/wifi_glue.rs:36-43`：每线程一个 ax-task，体为无限 `block_on(poll_fn(...))`。

| 线程 | 文件 | 职责 | 唤醒源 |
|---|---|---|---|
| wifi-rx | `fdrv/thread/rx.rs:61-106` | CARD_INT 后读 FIFO、分发数据/CFM | `irq_waker`（ISR + kicker + TX/CMD 侧） |
| wifi-rx-kick | `rx.rs:114-138` | 每 10ms 唤醒 rx（丢唤醒兜底） | `sleep_ms(10)` |
| wifi-tx | `fdrv/thread/tx.rs:45-82` | 批处理发送数据/命令帧 | `wake_pollset`（新帧入队）+ 10ms kicker（DC/DW） |
| wifi-ap | `fdrv/thread/ap.rs:23-80` | 关联队列、STA 删除、控制端口对账 | `assoc_pollset` + 周期 self-wake |

### 2.2 唤醒协议（现状的正确性骨架）

- **ISR → RX**：`sdio1_irq_handler`（`fdrv/core/bus.rs:316-326`）只做 `mask_card_irq()` + `irq_pending.store(Release)` + `irq_waker.wake()`（AtomicWaker，无锁无分配）。RX 侧"注册 waker → unmask → 双检"（`rx.rs:83-103`）闭合丢唤醒窗口。
- **任务间**：`PollSet`（vendored，无 sticky 状态，`fdrv/core/pollset.rs`）+ 双检协议（如 CFM 等待 `fdrv/protocol/cmd.rs:166-192`）。
- **关键缺口**：**信用（flow control）事件没有生产者**。fc 寄存器被 TX 轮询读取，但"信用恢复"这一事实从不唤醒任何 waker——TX 只有两个唤醒源：新帧入队（与信用无关）与 10ms kicker。

### 2.3 三处轮询病灶的精确位置

1. `tx.rs:255-268` `check_data_flow_control`：50 × (CMD52 读 + `yield_now`)；
2. `sdio_transport.rs:270-289` `wait_flow_ctrl` / `wait_flow_ctrl_for_size`：10 次重试的 yield 循环（CMD/EAPOL 路径）；
3. `sdio_transport.rs:214-239` `wakeup()`：V3 芯片唤醒轮询 sleep-ready 位（最多 200 × yield，低优先改造）。

---

## 3. 既有异步驱动调研

### 3.1 USB 栈（全仓库唯一完整异步驱动栈）

- **每传输一个 Future**：xhci `TWaiter`（`drivers/usb/usb-host/src/backend/kmod/queue.rs:147-169`）——检查完成槽 → `AtomicWaker::register` → 再检查 → Pending；完成路径只做"锁存状态到原子 + `waker.wake()`"（queue.rs:220）。与 aic8800 现有 ISR→RX 协议同构。
- **执行模型**：TRB-as-task、executor-agnostic（`drivers/usb/CLAUDE.md`、`docs/docs/architecture/driver/usb/design.md:6-16`）；StarryOS usbfs 用 `ax_task::future::block_on(poll_fn(...))` 驱动（`os/StarryOS/kernel/src/pseudofs/usbfs/manager.rs:214-218`）。
- **IRQ/轮询双模**：有 IRQ 走事件泵（`usbfs/irq.rs:96-102,217-285`）；无 IRQ 自动降级为轮询任务（`irq.rs:120-143` `usbfs-event-pump`）。这为 aic8800 的 kicker 提供了设计范本：**kicker 应是"无事件源时的降级模式"，而不是常态修复手段**。

### 3.2 块 MQ runtime（流水线范本）

- `HardwareQueue` 三段式（`drivers/interface/rdif-block/src/hardware.rs:71-146`）：`submit_batch_owned → commit_submissions`（每批一次门铃）`→ drain_completions`（**仅在有已确认 IRQ 事件后调用，文档明说不是轮询 API**）。
- hctx 维护任务循环（`fs/ax-fs-ng/src/block/runtime/hctx/mod.rs:255-371`）：排空 IRQ 锁存 → 定时器重试 → 提交 → 带 deadline 睡眠。**IRQ 唤醒任务，任务推进流程**。
- 软件流水线：`SOFTWARE_PIPELINE_WINDOWS = 2`（`fs/ax-fs-ng/src/block/runtime/lifecycle/io.rs:16,29-60`）——第 N 窗口未完成即提交第 N+1 窗口。设计文档：`docs/design/block-mq-runtime.md:43-52,314-322`。

### 3.3 rd-net（管道保满机制）

- 每队列 `AtomicWaker` 映射（`drivers/net/rd-net/src/lib.rs:16-40`）；IRQ 上下分离：`handle_irq` 只 ack + 发布事件位，`handle` 在任务上下文唤醒 waker（lib.rs:280-302）。
- TX：`reclaim_bounded` 先回收再提交（lib.rs:333-346）——管道有界。
- RX：`prefill` 整环填满 + `RxPacket::consume` 回调后**立即回注 buffer**（lib.rs:436-447,514-521）——管道保满。

### 3.4 axtask::future 能力清单

模块 `os/arceos/modules/axtask/src/future/`（导出为 `ax_task::future`）：

- `block_on`（mod.rs:100）、`interruptible`（mod.rs:148）；
- `poll_io`（poll.rs:22，Pollable 注册 + 注册后重试，EINPROGRESS 安全）；
- **`register_irq_waker`**（poll.rs:79-171）：通用"future 等硬件 IRQ"桥——ISR 只置 pending 位 + `notify_irq`，`irq_waker_drain` 任务在 task 上下文唤醒 PollSet。**已实现但全树无调用者**；
- `sleep`/`sleep_until`/`timeout`/`timeout_at`（time.rs:129-176）；
- **无 join/select**（timeout 内部用 `futures_util::select_biased!`）。

### 3.5 ax-net 边界

- `net-poll` 任务（`net/ax-net/src/lib.rs:313,784-803`）：`NET_POLL_WAKE.wait_timeout_until(delay, || requested || irq_pending || deferred)`，无 IRQ 时退化为周期唤醒（`wake_all_devices` 兜底）。
- 每设备 RX worker（`net/ax-net/src/router.rs:1091-1110`）：批接收循环，压入共享队列后 `request_poll()`；背压时 `yield_now()`。
- aic8800 经 `set_rx_wake`（`drivers/interface/rdif-eth/src/lib.rs:249` → `wake_net_task_irq`，`os/arceos/modules/axruntime/src/devices.rs:155`）以带外方式驱动协议栈轮询。

### 3.6 调研结论

1. 仓库内**没有异步网卡接口**，但有完整的两套可借鉴架构：USB 的"每传输 Future + AtomicWaker"与块 MQ 的"队列三段式 + 维护任务 + 窗口流水线"；
2. aic8800 自己已是"半异步"：线程即 Future、ISR 零锁、CFM 等待已是标准 future。**跃进的实质不是引入 future，而是补齐事件生产端**——信用事件、完成事件——并消灭轮询；
3. `register_irq_waker` 是现成的通用桥，但 aic8800 的 CARD_INT 协议（mask/unmask + 双检）比它更精确，暂不替换。

### 3.7 厂商 Linux 驱动对照（2026-08-22 研究）

来源：sipeed/LicheeRV-Nano-Build，`osdrv/extdrv/wireless/aic8800/aic8800_fdrv/`（稀疏克隆至 /tmp 本地研读）。

1. **FC 寄存器语义**：non-V3 单寄存器 0x0A（mask 0x7F），读值直接作为 `fw_avail_bufcnt`（固件可用缓冲单元数，aicwf_sdio.c:1930）；数据路径与命令消息路径**共用同一计数器**（`aicwf_sdio_flow_ctrl` 要求 fc>2，`_msg` 版要求 fc!=0）——"TX 数据与 TX 命令共享一个信用池"在厂商代码中是明确事实。**注意：D80/D80X2 是 V3 芯片**（`is_v3()` 实现确认，common/mod.rs:141-143），TX 信用读 Q1（0x03）；Q2（0x09）在本驱动中定义但未使用——V3 按队列分流控的存在是对"池子可能分区"的提示，fc 共享判定实验（10.2）读 Q1 即可。
2. **等待方式**：`aicwf_sdio_flow_ctrl`（aicwf_sdio.c:351-391）= 50 次重试的**阻塞轮询**（udelay 200µs ×30 → msleep 2ms ×10 → msleep 10ms），在专用 TX 工作线程内执行。**厂商同样没有信用恢复中断**——实证了 5.4"固件路径无事件源"的判断（我们的 50 次 yield 正是其直译）。
3. **RX 不通知 TX**：`bustx_trgg`（TX 线程触发 completion）仅由帧入队（aicwf_sdio.c:2010）与消息发送（:2046）触发，RX 排水路径既不读 FC 也不唤醒 TX。**Phase 1 的"RX 排水→查 fc→敲 TX 门铃"是超越厂商的改进**（前提：共享池假设成立）。
4. **TX 聚合（厂商的流水线机制）**：fc 值即聚合预算——`aggr_count == fw_avail_bufcnt - THRESH` 时一次 `aicwf_sdio_aggr_send` 多帧单次 CMD53（aicwf_sdio.c:2095-2098）。我们的 Rust 驱动每帧一次 `write_fifo`，无聚合 → 见 5.7 新增候选。
5. **RX 预分配池**：aicwf_rx_prealloc.c——固定大小 `rx_buff` 链表池（get/free 带自旋锁），排水直接取用不现分配。
6. **兜底粒度**：IRQ/RX 线程超时兜底 `rx_thread_wait_to = 1000ms`（aicwf_sdio.c:2433）——厂商兜底是 1 秒级，我们的 10ms kicker 已细 100 倍，Phase 1 事件化后更细。

### 3.8 既往优化报告对照（../sg2002-wifi.md，同代码库 2026-07 实测）

报告作者 = 本 Rust 移植作者，Part 1（0.2M→10M，SDHCI 忙等）+ Part 2（10M→13.7M/18.9M）。

1. **Part 1 根因与修复已被本 PR 取代**：根因是等待期 `yield_now()` 撞 sched-rr 50ms 时间片（每笔写放大 ~48ms）；修复为 3ms 时间上界忙等。其"不得体"之处：等待期**占 CPU + 全程持锁**（每笔 ~212µs 总线独占）。本 PR 的 XFER 中断 + 防丢唤醒协议正是其语义改进：等待期释放 CPU（Phase 2b 后进一步释放总线）。
2. **Part 2 优化项与当前分支对照**（2026-08-22 源码核实）：

| 优化项 | 报告实测效果 | 当前分支状态 |
|---|---|---|
| HT 结构体对齐（26→32/54→56/102→112） | 聚合 0→5-10× | 未逐一核对（常量命名不同） |
| TX/RX kicker 10ms→1ms | **+20%**（10.6→12.7M） | **未包含**：tx.rs:107 / rx.rs:132 仍 10ms |
| SDIO 25→50MHz + PHY delay | **下行 +96%**（7→13.7M） | **未包含**：regs.rs:150 仍 25MHz，PHY delay 序列未配置 |
| fc 空转 50×→1 次+yield | fcrd 40-54%→**0.4%**，6M→12M 恢复 | **未包含**：tx.rs:256 仍 50× → **Phase 1 结构性取代此项** |

3. **Phase 1 的量化动机**：50MHz 下 fcrd（流控读耗时）占 TX 窗口 **40-54%**——这正是"流控轮询烧总线"的直接测量；Phase 1 消灭该轮询的价值上限由此锚定。
4. **报告遗留差距与厂商机制互相印证**：HE（~60%）、DMA/ADMA2（~30%）、**多帧拼包（~10%，即厂商 aicwf_sdio_aggr 机制）**——第三项与 3.7.4 同源，列为 5.7。

---

## 4. 设计原则（正确性优先）

取自 `.claude/skills/cross-kernel-driver/references/architecture.md`：

1. **中断只同步状态，任务才推进流程**：ISR 只 ack/置位/计数，不排水、不持锁、不分配；
2. **事件必须有人认领**：任何"等待某条件"之处，必须能指出该条件的**事件生产者**；找不到生产者就显式声明"无事件源，用有界超时轮询"，并把超时周期写清楚（这是当前 fc 等待缺的那一块）；
3. **注册-双检协议**：消费者注册 waker 后必须重检条件，生产者唤醒允许落空——事件由"锁存状态"而非"敲门声"承载（aic8800 现状已遵循）；
4. **锁的作用域必须短**：不持锁跨越阻塞等待（代码质量准则 10.2 同义）；
5. **边界锁保持同步**：被 ax-net 在 irqsave 下触达的锁不得异步化（红线 1.2）。

---

## 5. Phase 1：TX 信用事件（低风险、最大收益）

### 5.1 问题

TX 批处理中 fc 不足时的三种等待方式（50 次 yield 轮询、10 次 yield 轮询、干等 kicker）都**不认识"信用恢复"这一事件**。

### 5.2 事件生产与消费

**生产者（RX 线程，task 上下文）**：RX 每排空一批 FIFO 后，无条件做一次 fc 检查（一次 CMD52，微秒级）；若 `fc * BUFFER_SIZE` 满足待发帧长度，`tx.wake_pollset.wake()`。理由：RX 排空释放卡侧共享缓冲池，是信用回升的**确定性触发点**之一。

**消费者（TX 线程）**：fc 不足时不再自旋，改为：`wake_pollset.register(cx.waker())` → **重检 fc** → 仍不足则 Pending；帧入队路径（现有 `tx.rs:562,586`）继续唤醒，无需改动。

### 5.3 丢唤醒分析（三窗口）

| 时序 | 结果 |
|---|---|
| RX 检查+唤醒发生在 TX 检查 fc **之前** | TX 检查直接看到充足信用，不睡 |
| RX 唤醒发生在 TX"检查后、注册前" | 唤醒落空，但 TX 注册后的**重检**看到充足信用，不睡 |
| RX 唤醒发生在 TX 注册**之后** | 唤醒命中，重检通过 |

三窗口全覆盖，与 rx.rs:83-103 的既有协议完全同构，无需引入新同步原语。

### 5.4 诚实声明：固件侧信用补充无事件源

fc 的另一回升路径是**固件异步处理 TX 帧后释放缓冲**——本仓库无证据表明它伴随中断信号（无此方面的硬件文档依据）。因此设计保留**有界周期重检**作为第二唤醒源：

- 方案 A（首选）：TX 信用等待 = `wake_pollset` + 短超时重检（起始周期取 **1ms**——报告实测 10→1ms 带来 +20%，见 3.8；最终值由板测决定）；kicker 保留为纯安全网；
- 方案 B：沿用 10ms kicker 但增加"唤醒时重检 fc"分支——改动更小，收益打折。

无论选哪，都**消灭了 50 次 yield 自旋**（CPU 让出方式从"忙让"变"睡眠"），且 RX 排水驱动的唤醒覆盖共享缓冲池回升的常见情形。

### 5.5 边界条件

- **RX 空闲**（无数据可排）：信用只靠固件路径回升 → 由 5.4 的超时重检覆盖；
- **shutdown**：`WifiBus::shutdown`（bus.rs:247-276）已唤醒 `wake_pollset`，TX 重检时看到 `BusState::Down` 退出，无新增风险；
- **虚假唤醒**：无害，TX 重检 fc 后重新注册。

### 5.6 改动面

`tx.rs`（check_data_flow_control 改为单次检查 + 事件等待）、`rx.rs`（排水后信用检查 + 唤醒）、`sdio_transport.rs`（wait_flow_ctrl* 供 CMD/EAPOL 路径复用事件等待）。**不触碰 sdhci、不触碰 IRQ、不触碰边界锁。**

### 5.7 并行候选：TX 聚合（厂商机制，Phase 1.5）

厂商的流水线核心是**聚合**：把 fc 值当预算，一次 CMD53 写多帧（3.7.4）；报告亦将"多帧拼包"列为 ~10% 差距项（3.8.4）。当前 Rust 驱动每帧一次 `write_fifo`，每帧付一次 CMD53 建立/完成等待开销。候选实现：`process_data_tx` 批内按 `fc - THRESH` 预算攒帧进聚合缓冲，单次 `write_fifo` 发出。与 Phase 1 正交可叠加（Phase 1 提供"何时写"，聚合提供"写多少"），建议在 Phase 1 板测数据出来后作为 Phase 1.5 立项。正确性注意点：聚合后的帧长需对齐检查（厂商按 BUFFER_SIZE/TAIL 语义逐帧拼接），且一次写多帧会一次性消耗多单位信用，fc 预算读取与写入之间需保持单次闸门检查（与 Phase 1 的"单次检查"语义一致）。

---

## 6. Phase 2：完成所有权令牌 + 计数等待（中风险）

### 6.1 问题

transport 锁跨 Phase 2 阻塞等待（`sdio_transport.rs:166-173`）。持锁睡眠违反调度器契约（深度 3 断言隐患，`run_queue.rs:966-974`）；且 UP 上 SpinLock 互斥溶解后，另一任务可在等待期误入总线、破坏单 waiter 完成协议。

### 6.2 被否决的方案：计数器 + 等待期并发数据段

最初设想"释放锁后 rx 可在 tx 等待期发起自己的数据传输，用 `xfer_seq` 快照区分归属"。深入分析后否决，原因是**共享 XFER_COMPLETE latch 没有 per-transfer 身份**：

反例：tx 进入 Phase 2 释放锁（其数据段可能仍在途）→ rx 取得锁、做 sweep（未见位）、取快照 `snap_rx` → rx 在 `wait_data_idle` 自旋（等 tx 数据段结束）→ tx 的完成在 rx 快照**之后**锁存 → ISR 认领（seq 自增）→ rx 的条件 `seq > snap_rx` 被 **tx 的完成**满足 → rx 虚假早醒，把自己的在途传输误判为完成。

根因：`wait_data_idle` 通过（DATA_INHIBIT 清除）与 XFER_COMPLETE 锁存之间存在硬件滞后，快照无法获知"还有几笔完成的认领在途"。票号数学（`ticket = seq + k`）对 k 的判定不可靠。**结论：SDHCI 硬件只支持深度一的数据段串行，设计必须把它形式化，而不是绕过去。**

### 6.3 不变量（改造后必须逐条保持）

- **I1（硬件串行）**：同一时刻至多一个数据段在途——`wait_data_idle` 的 DATA_INHIBIT 硬件门控；
- **I4（完成所有权令牌，新增）**：`completion_owner` 令牌（transport 层）——任一时刻至多一笔数据段"在途或待认领"。数据段发起前必须取得令牌（前一笔的完成已认领才可取得），完成认领后释放。令牌把 I1 从"硬件事实"升级为"软件协议"，是等待期释放锁而协议不破的前提；
- **I2（完成身份）**：持令牌、无他人在途的时刻取快照 `snap = xfer_seq`，之后第一次自增**必属本传输**（由 I4 + I1 推得，6.2 的反例被 I4 结构性排除）；
- **I3（无残留锁存）**：每笔完成被认领时即 W1C，位不残留；`clear_stale_status` 对 XFER_COMPLETE 的**保留**逻辑（`lib.rs:344-358`）**继续需要**——锁存后、ISR 认领前存在窗口，命令路径的 stale 清理不得误清该位（`f727d4123` 的教训在新协议下同样成立）。

### 6.4 协议设计

**ISR 职责变更**（`drivers/blk/sdhci-cv1800/src/irq.rs:210-217` 的 XFER_COMPLETE 路径）：

```
mask XFER 信号（不变）
若 sticky 位置位：W1C 清位 + xfer_seq.fetch_add(1, Release)   ← 认领：ISR 是唯一认领者
pio_wake_callback()（不变，notify 等待队列）
```

sticky 位"事件记忆"的角色迁移到 `xfer_seq`（原子计数，不依赖通知送达）。Phase 1 快路径的任务侧 W1C 保留（幂等，纯观察不清计数）。

**等待方协议**（transport 层 + `poll_int_status` Phase 2 改造）：

```
发起数据段前（持有 transport 锁）：
  取得完成令牌（等待前一笔认领完毕）        ← I4

wait_transfer_complete：
  Phase 1 自旋原样（快路径；若 ISR 抢先清位则本段落空，无害）
  Phase 1 未命中：
    snap = xfer_seq.load(Acquire)           ← I2：此刻令牌在手，下一次自增必属本传输
    释放 transport 锁                        ← 等待期允许他方 CMD52 交错（流控读等）
    block_timeout_until(10ms, || xfer_seq > snap)
    重新获取 transport 锁
    seq > snap → 释放令牌、返回 Ok（不读、不清 sticky 位）
    否则（超时）→ 错误检查 + DAT 复位（见 6.5.4）→ 释放令牌
```

注意：`wait_timeout_until` 的"锁内条件检查 + 入队衔接"（`os/arceos/modules/axtask/src/wait_queue.rs:153-184`）原样保留——丢唤醒防护性质不变，条件从"读位"换成"读计数"。等待期他方 CMD52 只锁存 CMD_COMPLETE，不触碰 XFER_COMPLETE/令牌，与等待协议无交互。

### 6.5 逐项竞态分析

1. **完成发生在 Phase 1 自旋期间（ISR 抢先清位）**：位锁存 → ISR（mask + 认领）→ Phase 1 下一次读看不到位 → 快路径落空。但快照先于 Phase 1 取得，`xfer_seq` 已大于 `snap` → Phase 2 锁内条件检查立即命中，**零睡眠返回**。无丢失窗口。
2. **完成发生在 Phase 1 之后、入队之前**：ISR 认领 → 条件在锁内检查时成立 → 不睡。✓
3. **完成发生在入队之后**：notify 唤醒 → 醒后重检条件 → 成立。✓
4. **错误路径**（错误不产生中断）：本传输出错 → 计数不自增 → 等待 10ms 超时 → 醒后重新持锁，再做错误检查与 DAT 复位（`poll_status_once` 的错误分支整体保留，且**必须在重新持锁之后**执行——此时令牌在手，在途者只可能是自己，共享错误位的归属无歧义）。代价：错误路径退化 10ms，与现状"睡期中段错误由 post-wake 重检查出"语义一致，可接受。
5. **stale 位泄漏**：ISR 每次认领即 W1C，位不残留——`f727d4123` 修复的"stale 位导致虚假过早成功"整类问题被结构性消除。
6. **SIG_EN RMW 竞态自愈**：不变（stale RMW → 最多一次多余 ISR → mask + 无位可认领 + notify，无害）。
7. **超时后的 DAT-busy 焊死防护**：保留 `poll_int_status` 现有超时分支（选择性清位 + `reset_dat_line`），与 4 同路径执行。
8. **等待期 CMD52 交错**：rx 的流控读/唤醒查询在 tx 等待期照常执行（单核 ISR 只插入在指令间，认领不干扰 CMD52 状态机）。✓

### 6.6 API 层面影响（必须明示）

"等待期间释放锁"无法在现 API 形态下实现：guard 由 `SdioTransport::write_fifo` 在**调用前**创建（`sdio_transport.rs:172`），sdhci 内部拿不到它。需要把 SDIO 数据传输 API 拆成两段（issue/pio 与 await-completion），或引入完成句柄；令牌作为 transport 层的并发结构新增。改动触及 `sdio-host` trait 及其实现（sdhci-cv1800）、aic8800 transport 层——**这是 Phase 2 的主要成本，也是把它独立成阶段的原因**。

### 6.7 收益重估与分级

收益重估（较初版设想下调）：等待期**并发数据段被否决**（6.2），可获取的收益是：

1. **消除持锁睡眠契约违反**——深度 3 断言隐患消失（确定性收益，主要动机）；
2. **完成认领协议形式化**——互斥溶解导致的协议破坏被令牌堵死（确定性收益）；
3. **等待期 CMD52 交错**——流控读不必排在整笔传输之后（流量相关，幅度未知）。

因此分级：

- **Phase 2a（建议先做）**：完成计数器 + ISR 认领清位 + 令牌 + API 拆分，**保持等待期持锁**。收益：1、2 落地，为 2b 铺路；风险低；
- **Phase 2b（实验）**：在 2a 之上释放锁等待。板测对照三方向吞吐与 Phase 2 退化次数；若无增益，放弃 2b 保留 2a（收益 1、2 不受影响）。

### 6.8 与锁型决策的耦合（重要结论）

Phase 2 落地后，transport 锁不再跨阻塞等待，且该锁**只被内部线程在 task 上下文获取**（ax-net 的 irqsave 链只触达 data_queue/tx.queue/sta_mac，见 1.2）——即 transport 锁是"可证明不经 irqsave 路径"的锁。这意味着：

- 睡眠 Mutex 对 transport 锁**重新变得合法**（m1.log 的排除结论只适用于边界锁）；
- Phase 2b 的"醒后重新获取"需要真实互斥（UP 上 SpinLock 无原子，互斥溶解），Mutex 是候选；
- 最终锁布局建议：**边界锁（data_queue/tx.queue/sta_mac 等）= 无上下文变化锁（raw/SpinRaw 语义）**；**transport 锁 = 短临界区 + 若 Phase 2b 落地则 Mutex**。锁型与异步化在各自阶段独立决策，互不阻塞。

---

## 7. Phase 3：kicker 降级与收编

1. ① ② 落地后，唤醒全覆盖：CARD_INT→RX、完成事件→发起方、信用事件→TX、CFM→cmd、assoc→ap。kicker 从"修复丢失唤醒的主力"降级为纯安全网；
2. 参考 usbfs 的"IRQ/轮询双模"（`os/StarryOS/kernel/src/pseudofs/usbfs/irq.rs:120-143`）：kicker 保留但**统计触发次数**（观测指标），触发次数趋近于零即验证事件覆盖完备；
3. 可选收编：`register_irq_waker`（`axtask/src/future/poll.rs:79`）作为通用 IRQ 等待桥，在 CARD_INT 协议若出现第二个消费者时再引入。

---

## 8. 边界锁分类表（异步化时的处置清单）

| 锁 | 位置 | 是否在 ax-net irqsave 链上 | 处置 |
|---|---|---|---|
| rx.data_queue | bus.rs:110 / device.rs:294 | 是（recv→reclaim） | 保持同步短临界区，锁型见 6.8 |
| tx.queue | bus.rs:141 / tx.rs:548-564 | 是（send→enqueue） | 同上 |
| conn.sta_mac / ap_mac | bus.rs:35-36 / ethernet.rs:276 | 是（mac_address） | 同上 |
| cmd.rsp_queue / pending | bus.rs:77-81 | 否（线程侧 CFM 匹配） | 可异步化（已是 future 形态） |
| rx.eapol_queue / tx.ind_queue / ap 各队列 | bus.rs:112-115,144-145,176-188 | 否 | 可异步化 |
| transport（sdio） | sdio_transport.rs:28 | 否（仅内部线程） | Phase 2 后短临界区；Mutex 合法 |
| irq_waker / irq_pending | bus.rs:108-109 | ISR 侧 | AtomicWaker，不变 |

（"是否在 irqsave 链上"依据：`net/ax-net/src/device/ethernet.rs:276,620-624` 的调用链 + m1.log 实测现场；实现时以 Mutex 的 might_sleep 板测作运行时复核。）

---

## 9. 风险登记表

| # | 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|---|
| R1 | 信用事件生产不完整（固件路径无事件源） | 高 | TX 等待退化为周期重检 | 5.4 的超时兜底 + kicker 安全网 |
| R2 | Phase 2 快照协议实现错误 | 中 | 丢唤醒（10ms 退化）或误醒（消费他位） | 6.5 逐项分析 + 回归测试重放（见 10） |
| R3 | Phase 2b 无收益 | 中 | 复杂度白付 | 6.7 分级：2a 先行，2b 板测对照后定夺 |
| R4 | 边界锁误异步化 | 低 | m1.log 式启动 panic | 8 分类表 + might_sleep 板测复核 |
| R5 | 单核流水线收益上限低于预期 | 中 | 目标落空 | 以三方向 iperf + kicker 触发计数为验收指标，数据说话 |
| R6 | sdhci API 拆分引入回归 | 中 | 传输路径 bug | Phase 2a 先行验证计数器协议，再动 API |

---

## 10. 验证计划

### 10.1 确定性回归测试（先于板测）

- **信用事件竞态**：扩展 `FakeIrqDelay` 思路（`drivers/blk/sdhci-cv1800/src/lib.rs:932-1030` 的既有重放模式）：在 TX"检查 fc 后、注册 waker 前"窗口内重放"RX 排水 + 唤醒"，断言 TX 不睡眠（零 `sleep_ms`）且批处理继续；
- **完成计数竞态**：三种重放——ISR 在快照前（sticky 已清、计数已含，Phase 1 落空但锁内条件立即命中）、在快照后入队前、在入队后；断言等待零退化睡眠、不误消费他人完成；
- **错误路径**：错误锁存时计数不推进 → 10ms 超时 → 醒后持锁消费错误 + DAT 复位，断言与现状语义一致。

### 10.2 板测（sg2002，三方向 iperf 对照）

- 对照基线：`www/logs/irq-rc1.log`（11.2/12.0 上行、11.0 下行、5.37/5.81 双向）；
- 每阶段产物各测一轮：Phase 1、Phase 2a、Phase 2b；
- 观测指标：三方向吞吐、`data flow ctrl timeout` 告警数、kicker 触发计数（Phase 3 起）、Phase 2 退化次数；
- **基线次序提示**：3.8 表所列报告项（kicker 1ms、50MHz+PHY delay）当前分支未包含，且是已验证的独立杠杆——建议先决定是否补回这两项再测 Phase 1，避免在 25MHz/10ms 基线上测出的 Phase 1 增益与补回项混叠。

### 10.3 构建与工具

`cargo fmt -p aic8800`、`cargo xtask clippy --package aic8800`（-D warnings）、
`cargo xtask starry build -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml`。

---

## 11. 参考

- `www/sg2002-wifi-irq-rebase-perf-regression-20260821.md`（回归根因与实验数据）
- `docs/design/block-mq-runtime.md`（流水线参考模型）
- `docs/docs/architecture/driver/usb/design.md`、`drivers/usb/CLAUDE.md`（异步驱动范本）
- `.claude/skills/cross-kernel-driver/references/architecture.md`（IRQ/队列所有权规则）
- `../sg2002-wifi.md`（同代码库既往优化报告：Part 1 忙等根因、Part 2 四项优化，见 3.8）
- 厂商 Linux 驱动：sipeed/LicheeRV-Nano-Build `osdrv/extdrv/wireless/aic8800/aic8800_fdrv/`（见 3.7）
- 行号锚点均出自当前 HEAD（3d9313301）源码。
