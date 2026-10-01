# 第 8 章 owner 的转移与运行时的配合

> 对照代码：
> - `drivers/net/aic8800/src/rdif/device/endpoints/device.rs`（into_parts 总装配）
> - `drivers/net/aic8800/src/rdif/device/endpoints/startup.rs`（owner 移交与 poll 控制）
> - `drivers/net/aic8800/src/rdif/owner/progress.rs`（AicOwner 本体）
> - `net/ax-net/src/queue_runtime/executor/mod.rs`（initialize/poll/等待）、`executor/wifi.rs`（控制步骤）
> - `net/ax-net/src/queue_runtime/state.rs`（启动期 IRQ 语义）
> - `drivers/blk/sdmmc-host/src/host/mod.rs`（事务契约）

这是收束章节：前面每一章的机制，最终都要挂到"唯一的 owner"和"网络运行时"这两个锚点上。

## 8.1 谁拥有硬件：AicOwner

`AicOwner`（`rdif/owner/progress.rs`）持有全部"只能有一个持有者"的东西：

```rust
pub(crate) struct AicOwner<H: SdMmcIrqHost + 'static> {
    card: SdioCard<H>,            // 卡协议状态机 + 总线访问
    card_irq: Option<H::CardIrq>, // CARD_INT 的 mask/rearm 端点
    init: Option<SdioInitRequest<H>>, // 卡初始化事务
    device: AicDevice,            // 第 2 章的纯核心
    active: Option<ActiveOperation<H>>, // 正在飞的那个 SDIO 操作
    // + 各道环的端点、IRQ latch、MAC 发布
}
```

它**不是线程**、不是任务，就是一个数据结构。网络运行时的 executor 任务在固定 CPU 上调用它的方法，每次调用推进一步（第 1 章的原则也适用于 owner 这一层：`advance_with_cause` 每轮最多 16 步，`OWNER_STEP_BUDGET`）。没有第二个实体能碰 `SdioCard` 或 `AicDevice`——它们被包在 owner 里，owner 只有一个。

## 8.2 两次移交：启动 → 就绪，靠一道容量 1 的环

`into_parts`（`endpoints/device.rs`）把 owner 交给 `AicOwnerStartup`（启动端点），同时建好 `OwnerChannels`：

```rust
let ring = HeapRb::new(1);   // 容量 1 的环（rdif/device/shared.rs）
let (sender, receiver) = ring.split();
```

启动阶段，`AicOwnerStartup` 持有 owner（`owner: Option<AicOwner<H>>`），运行时通过 `NetOwnerStartup::start/advance` 推进它。启动完成（`AicEvent::Started`）后：

```rust
// startup.rs 的 finish()
OwnerProgress::Ready => {
    transfer_owner(&mut self.owner, |owner| owner_sender.try_push(owner).err())
        .map_err(...)?;
    Ok(NetOwnerStartupProgress::Ready)
}
```

owner 被**推进**环里，启动端点自己的 `owner` 槽变 None。此后 `AicPollIrqControl`（poll 端点，`rearm_and_check/quiesce/shutdown` 的实现者）从环的另一端取出：

```rust
fn owner(&mut self) -> Result<&mut AicOwner<H>, NetError> {
    if self.owner.is_none() {
        self.owner = self.owner_receiver.try_pop();   // 就绪后第一次调用时接管
    }
    self.owner.as_mut().ok_or(NetError::Stopped)
}
```

为什么用环而不是 `Arc<Mutex>`：移交是**一次性的所有权转移**，不是共享。环的语义恰好就是"恰好一个槽、放进去之后永远只有一个端能拿出来"。`transfer_owner` 失败时把 owner 放回原槽（`TransferOwnerError::Rejected` 路径），保证移交失败不丢 owner。没有锁、没有引用计数、没有"两个端点同时看到 owner"的任何可能。

## 8.3 启动期：poll group 关着，但 IRQ 必须能叫醒启动

启动期间队列尚未发布，group 处于 DISABLED（`state.rs` 的初始态）。但启动状态机（固件上传、LMAC 启动）**依赖中断**——SDIO 事务的完成中断、固件的 CARD_INT。看 `state.rs` 的 `schedule_irq` 怎么处理这个矛盾：

```rust
if self.is_disabled() {
    // During owner startup queues stay disabled, but the startup
    // state machine still needs the IRQ notification to advance.
    self.notify.notify_irq();
} else if self.publish_schedule() {
    self.notify.notify_irq();
}
```

DISABLED 时中断不走"调度 poll"路径（队列还没开放），但**仍然敲醒正在等的 executor**。配合 `executor/mod.rs` 的 `initialize`：

```rust
let mut progress = startup.start(now_nanos);
loop {
    progress = match progress {
        Ok(NetOwnerStartupProgress::Ready) => break,
        Ok(NetOwnerStartupProgress::WaitForInterrupt) => {
            self.shared.wait_startup_irq();          // 等中断（无 deadline）
            startup.advance(now_nanos)
        }
        Ok(NetOwnerStartupProgress::WaitForInterruptUntil { deadline_nanos }) => {
            self.shared.wait_startup_deadline(deadline_nanos);  // 等中断或等时刻
            startup.advance(now_nanos)
        }
        Ok(NetOwnerStartupProgress::RetryAt { deadline_nanos }) => {
            self.shared.wait_startup_deadline(deadline_nanos);  // 等到时刻
            startup.advance(now_nanos)
        }
        Err(error) => { let _ = startup.cancel(); return Err(error); }
    };
}
```

`wait_startup_deadline` 就是 `notify.wait_timeout(duration)`——**一次带超时的等待同时覆盖"等中断"和"等到点"**：中断来了提前醒（没到 deadline 继续等），deadline 到了也醒。这就是第 1 章"等待由运行时执行"的最直接体现：核心报出 deadline，运行时把它翻译成一次 wait_timeout。

启动完成后的收尾（`initialize` 末尾）：分配 TX 池、`rx.initial_refill`、第一次 `rearm_and_check`，然后 `shared.activate(pending)` 把 group 从 DISABLED 翻到 IDLE/SCHEDULED——**从这一刻起队列才向协议层开放**。

## 8.4 就绪后：poll 与 rearm 的配合

就绪后 owner 归 `AicPollIrqControl`。ax-net 每次 poll 一个 group 走完 driver 侧步骤后调 `rearm_and_check(now_nanos)`。AIC 的实现在 `owner/progress.rs:140` 的 `rearm_and_advance`：

```rust
pub(crate) fn rearm_and_advance(&mut self, now_nanos: u64)
    -> Result<(OwnerProgress, bool), AicRdifError> {
    let progress = self.advance_with_rearm(now_nanos, false)?;   // 先尽量推进（不 rearm）
    let card_pending = self.card_irq.as_mut()
        .is_some_and(CardIrqControl::rearm_and_check);           // 再原子重开 CARD_INT
    self.card.host_mut().enable_completion_irq()?;               // 重开事务完成中断
    Ok((progress,
        card_pending || self.irq_latch.has_pending() || self.outputs.has_runnable_pending()))
}
```

顺序是刻意的：**先推进到无活可干，再原子地重开中断并复查**。`rearm_and_check`（`sdhci-host` 的 `SdhciCardIrqHandle`）做的是第 5 章讲过的电平闭合：开信号 → 内存屏障 → 读状态 → 仍挂着就重新 mask 并返回 true。重开后三处复查——控制器级（card_pending）、latch 级（中断已到达但没消费）、输出级（环上有扣住待发的数据）——任何一处有货都报 `WorkPending`，executor 立即再排一轮（`finish_idle` 里 `WorkPending → schedule_task`）。

返回值还可能携带 `RetryAt`（核心在等 1ms 信用重试等）。`executor/mod.rs` 的 `finish_idle` 把它记进 `group.retry_at`，主循环的休眠决策里：

```rust
let deadline_nanos = wifi.iter().filter_map(WifiExecutorSlot::deadline)
    .chain(groups.iter().filter_map(|group| group.retry_at))
    .min();
// ExecutorWait::Deadline(duration) → notify.wait_timeout(duration)
```

**group 的休眠永远不"睡死"**：有 retry_at 就带超时睡，时刻到了自然醒。

## 8.5 sdmmc 事务契约速览

owner 每发一个 SDIO 请求，`ActiveOperation::submit/advance/abort`（`rdif/owner/operation.rs`）在底层对应 sdmmc 协议的一条事务。这份契约的关键点（`sdmmc-host/src/host/mod.rs`）：

- **单活动事务**：一条总线上同一时刻只有一个事务/总线操作，submit 可能被 `Busy` 拒绝（这就是第 2 章"单飞行"规则的硬件根源）。
- **`ProgressCause` 三值**：`Submitted`（刚提交，可启动硬件）、`AcknowledgedIrq`（中断已确认，可以推进到完成）、`RegisterRetry`（只允许寄存器级重试，**绝不允许**把 CMD/DAT 阶段推成完成）。
- **`RequestProgress` 三值**：`RegisterPending { retry_after }`（寄存器没就绪，过这个间隔再试）、`WaitingForIrq`（等中断）、`Complete(Result)`。
- **abort 的静默保证**：`abort_transaction` 返回（无论成败）时，控制器必须已停止命令/数据引擎和 DMA——这是"归还 buffer 之前证明硬件不再碰内存"的安全基石。
- **`ProgressCause::AcknowledgedIrq` 才能完成 CMD/DAT 阶段**——完成的事实必须来自硬件中断，不能靠软件猜。这正是第 5 章"合并事件快照同时推两件事"的协议侧另一半。

owner 的 `protocol_wait` 把 `RequestProgress` 翻译成 `OwnerWait`：`WaitingForIrq → Interrupt`，`RegisterPending { retry_after } → RetryAt(now + retry_after)`。

## 8.6 关闭与失败：回滚的完整链条

运行时决定停止时（设计文档 §启动、等待和回滚），顺序是：

1. **disable + synchronize IRQ**（运行时层）：先确保硬中断回调不再执行。
2. **cancel/abort**：`AicOwnerStartup::cancel` 或 `AicPollIrqControl::shutdown` → owner 的 `shutdown`（`owner/progress.rs:156`）：abort 活动操作、abort 初始化事务、`card_irq.disable()`、`disable_completion_irq()`。
3. **证明 DMA 停止才释放**：`shutdown_owner`（`startup.rs`）里的关键细节：

```rust
fn shutdown_owner<T, E>(slot: &mut Option<T>, shutdown: impl FnOnce(&mut T) -> Result<(), E>)
    -> Result<(), E> {
    let Some(mut owner) = slot.take() else { return Ok(()); };
    if let Err(error) = shutdown(&mut owner) {
        // Shutdown failure cannot prove that hardware stopped touching the
        // owner's backing, so preserve the entire ownership domain.
        core::mem::forget(owner);
        return Err(error);
    }
    Ok(())
}
```

关闭失败时 **`mem::forget(owner)`**——不是泄漏 bug，是刻意的隔离：无法证明硬件已停止访问 owner 持有的内存，就不能让 owner 的析构释放它们。保留整个 ownership domain（连同 DMA backing）比释放后硬件再写要好。这与核心 `fail`（第 2 章）"先归还全部 token 再报错"、适配层"buffer 只许原样退回"是同一原则的三处体现：**所有权在每一步都有明确的去处，任何失败路径都不允许所有权悬空。**

## 8.7 全景收束

把八章串起来看一张完整图：

```text
ax-net 运行时（固定 CPU 的 executor 任务）
 │
 ├─ 启动期：NetOwnerStartup::start/advance
 │     ← WaitForInterruptUntil / RetryAt（运行时转成 wait_timeout）
 │     → AicOwner：SdioInitRequest → AicDevice.start/advance
 │        → 21 阶段启动状态机（第 4 章）
 │        → 每阶段至多一个 SDIO 请求（单飞行，第 2 章）
 │        → debug/LMAC mailbox（第 3 章）
 │     → Started：owner 经容量 1 环移交（本章）
 │
 ├─ 就绪期：group poll（initialize 末尾 activate 开放队列）
 │     IRQ → SdhciIrqHandle → IrqLatch（第 5 章）
 │     executor claim → poll：
 │        TX：环① → 核心 Tx → 信用 → WR_FIFO → token 回环⑤（第 6 章）
 │        RX：irq_pending → 块数 → RD_FIFO → parse → 帧回 rx 环（第 5 章）
 │        Control：环 → 命令队列 → mailbox（第 7 章）
 │     rearm_and_advance：先推进，再原子重开 CARD_INT + 事务中断，复查三处（本章）
 │     group 休眠：永远带 deadline（retry_at / wifi deadline 的最小值）
 │
 └─ 关闭：disable+synchronize IRQ → abort 活动操作 → 证明停止才释放，
      失败则 mem::forget 隔离（本章）
```

这张图里的每一条边，都是"所有权或事实的单向流动"；每一个等待，都是"运行时的带超时 wait_timeout"；每一个失败，都有明确的归还或隔离路径。这就是重构后 aic8800 机制的全貌。

## 8.8 进一步阅读

- 设计文档：`docs/design/unified-sdio-aic8800.md`
- SD 卡协议侧：`drivers/blk/sdmmc-protocol/src/sdio/io/`（Function 生命周期、transfer）、`sdmmc-host/src/host/mod.rs`（事务契约）
- 控制器侧：`drivers/blk/sdhci-host/src/`、`drivers/blk/cv181x-sdhci/src/`
- 运行时侧：`net/ax-net/src/queue_runtime/{mod,state}.rs`、`executor/{mod,wifi}.rs`
- 平台接线：`drivers/ax-driver/src/net/aic8800/{mod,fdt}.rs`、`drivers/ax-driver/src/cv181x/mod.rs`
