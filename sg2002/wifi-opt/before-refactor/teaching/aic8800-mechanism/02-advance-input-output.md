# 第 2 章 AicDevice 的运转规则

> 对照代码：`drivers/net/aic8800/src/device/owner.rs`（状态结构）、`progress.rs`（advance 主体）、`model.rs`（全部输入输出类型）
> 上一章建立了原则（驱动不自己等），这一章看原则落地的精确规则。

## 2.1 AicDevice 内部有什么

`owner.rs` 把全部状态分成四块：

```rust
pub struct AicDevice {
    chip: ChipVariant,              // 芯片型号，决定寄存器布局和固件镜像
    registers: RegisterMap,         // 各功能寄存器的 SDIO 地址表（按型号选）
    lifecycle: LifecycleState,      // 生命周期状态 + 三个子状态机入口
    io: IoState,                    // SDIO 请求的飞行状态
    data: DataPlaneState,           // 数据面：事件队列、RX 队列、TX 队列、MAC
}

struct LifecycleState {
    state: AicState,                // Stopped / Starting / Ready / Stopping / Failed
    startup: Option<StartupState>,  // 启动状态机（第 4 章）
    mailbox: Option<MailboxState>,  // 命令邮箱状态机（第 3 章）
    control: Option<ControlState>,  // 控制命令队列（第 7 章）
    last_time: MonotonicTime,
    retry_at: Option<MonotonicTime>,// "到了这个时刻才继续"
    cancel_pending: bool,
}

struct IoState {
    pending: Option<PendingIo>,     // 正在飞行的那个 SDIO 请求（至多一个）
    next: Option<(IoPurpose, SdioRequestKind)>, // 排队等待发出的下一个请求
    next_request_id: u64,
    irq_pending: bool,              // 有中断待处理
    last_irq_sequence: u64,
}

struct DataPlaneState {
    events: VecDeque<AicEvent>,     // 对外事件的缓冲队列
    rx: RxState,                    // 收到的以太网帧队列（容量 256）
    tx: TxState,                    // 待发送帧队列（容量 128）
    active_tx: Option<ActiveTx>,    // 正在发送的那一帧
    mac_address: [u8; 6],
    interface_index: u8,            // 固件分配的接口号（0xFF = 无效）
    station_index: u8,              // 固件分配的站点号
}
```

理解它的最好方式是记住一条主线：**`io.pending` 里同时只会有一个请求在飞；所有其他想发起的 SDIO 操作都排在 `io.next` 里，等前一个完成。** 这个"单飞行"规则的由来见 2.4。

## 2.2 advance 的固定检查顺序

`progress.rs:21`，一次 `advance` 按下面的顺序做检查，**走到第一个"有结论"的步骤就返回**：

```rust
pub fn advance(&mut self, input: AicInput) -> AicAction {
    if let Err(error) = self.observe_time(input.now) {      // ① 时间必须单调不减
        return self.fail(error);
    }
    if let Some(event) = input.event                          // ② 处理外部事件
        && let Err(error) = self.consume_input(event, input.now) {
        return self.fail(error);
    }
    if let Some(event) = self.data.events.pop_front() {       // ③ 先交出缓冲的事件
        return AicAction::Event(event);
    }
    if self.lifecycle.cancel_pending                          // ④ 有取消请求？
        && let Some(pending) = &self.io.pending {
        return AicAction::AbortSdio { request_id: pending.id };
    }
    if self.io.pending.is_some() {                            // ⑤ 有请求在飞？
        return AicAction::WaitForInterrupt;                   //    只能等它完成
    }
    if let Some((purpose, kind)) = self.io.next.take() {      // ⑥ 排队的下一个请求？
        return self.emit(purpose, kind);                      //    发出去
    }
    if let Some(deadline) = self.lifecycle.retry_at {         // ⑦ 没到重试时刻？
        if input.now < deadline {
            return AicAction::RetryAt(deadline);              //    报出时刻，等
        }
        self.lifecycle.retry_at = None;                       //    到了，继续往下走
    }
    match self.lifecycle.state {                              // ⑧ 推进当前状态机
        AicState::Starting => self.drive_startup(input.now),  //    启动 / 就绪 / 关闭
        AicState::Ready => self.drive_ready(input.now),
        AicState::Stopping => self.drive_shutdown(),
        AicState::Stopped | AicState::Failed => AicAction::Idle,
    }
}
```

这个顺序本身就是教材，逐条读它的道理：

**① 时间检查**：状态机里到处都是 deadline 比较，时间往回跳会让"等 5 秒超时"变成"永远等不到"。`observe_time` 发现 `now < last_time` 直接 `fail`（`NonMonotonicTime`），宁可停机也不让逻辑建立在倒退的时间上。

**② 事件处理在推进之前**：外部喂进来的事件（SDIO 完成、中断快照、控制请求、TX 帧）必须先落袋。注意 `consume_input` 只是把事件**吸收进状态**（比如 `SdioCompletion` 匹配到 `io.pending` 并推进子状态机、`Irq` 置起 `irq_pending` 标志），它本身不产出 action。

**③ 事件队列优先**：吸收事件后可能立刻产生了对外的结果（帧收到了、命令完成了），这些结果排在 `data.events` 里，下一轮 advance 先吐出来。为什么不在吸收时直接返回？因为一次 `advance` 的返回值只有一个 action，而吸收一个事件可能同时产生多件事（例如一次 SDIO 完成既推进了 mailbox 又产出了一个命令完成事件）；队列把它们缓冲住，一次交一件。

**④ 取消优先于一切推进**：只要 `cancel_pending` 且有一个请求在飞，第一优先的事就是 `AbortSdio`。这是第 1 章"取消在任何一步生效"的实现——检查点在每一轮 advance 的最前面。

**⑤ 单飞行**：有请求在飞就什么都不能干，报 `WaitForInterrupt` 让外界等。注意：此刻再喂入别的输入也不会推进状态机——`consume_input` 已经消费过事件了（②），如果②没有完成那个飞行中的请求，这里就是"只能等"。

**⑥ 排队请求的出口**：`io.next` 是"上一个请求完成后，子状态机想发的下一个请求"。它总是在⑤确认没有飞行请求之后才发出。

**⑦ retry_at 是门**：所有"过 1ms 再试"的语义都落在这里。没到时刻就报 `RetryAt`，外界决定怎么等；到了时刻就清掉、放行到⑧。它同时作用于 mailbox 的流控重试、TX 的信用重试、启动状态机各阶段的延时。

**⑧ 状态机推进**：四个生命周期状态各有一个驱动函数。`Starting` 走启动状态机，`Ready` 走数据面 + 命令 + 控制，`Stopping` 走关闭流程，`Stopped/Failed` 无事可做。

## 2.3 输入事件怎么落袋

`progress.rs:68` 的 `consume_input` 按事件类型分发：

```rust
match event {
    AicInputEvent::Sdio(completion) => self.consume_completion(completion, now),
    AicInputEvent::Irq(snapshot) => {
        if let Some(error) = snapshot.error { return Err(AicError::Sdio(error)); }
        if snapshot.sequence > self.io.last_irq_sequence {    // 序列号防重复/防乱序
            self.io.last_irq_sequence = snapshot.sequence;
            self.io.irq_pending |= snapshot.card_interrupt;   // 只记"卡有话说"这个事实
        }
        Ok(())
    }
    AicInputEvent::Control(request) => self.consume_control(request, now),
    AicInputEvent::Tx { token, frame } => {
        if self.lifecycle.state != AicState::Ready { return Err(AicError::Busy); }
        self.data.tx.enqueue(token, frame).map_err(|_| AicError::TxQueueFull)
    }
}
```

几个要点：

- **中断快照只置一个布尔**。中断能携带的"卡要通信"这个事实非常小，核心只需要记住"有中断待处理"（`io.irq_pending = true`）。真正去读卡状态、排空 FIFO 的活，由⑧推进状态机时以普通 SDIO 请求的方式发出（第 5 章详述）。`sequence` 是 latch 生成的递增编号，用来丢弃迟到的旧快照。
- **SDIO 完成的校验严格**（`progress.rs:128` `consume_completion`）：先取出 `io.pending`，要求 `completion.request_id` 与它完全相等；不等就放回去并报 `CompletionMismatch`（整个设备失败）。因为单飞行规则下**任何不匹配都说明有 bug**，不存在"对不上就先放着"的合理情形。
- **TX 帧在非 Ready 状态被拒**（`Busy`）：数据面只在设备就绪后开放。

## 2.4 为什么同一时刻只有一个 SDIO 请求在飞

`IoState` 的结构（`pending` + `next`）强制了这一点。原因在底层硬件的契约里：`sdmmc-host` 的 `SdMmcHost` trait 声明"主机总线上同一时刻只有一个活动事务"（`drivers/blk/sdmmc-host/src/host/mod.rs`）：

```rust
/// The base contract is single active transaction: a host may reject a submit
/// with [`Error::Busy`] while another transaction or bus operation is active.
```

一条 SDIO 总线，一条 CMD 线一条 DAT 线，CMD52/CMD53 是串行执行的。允许两个请求同时飞，上层就必须处理 `Busy` 拒绝、处理乱序完成、处理"取消哪一个"。单飞行让这些复杂度全部消失：`pending` 只有一个槽，完成必须匹配它，取消就是中止它。

核心发出请求靠 `progress.rs:169` 的 `emit`：

```rust
pub(super) fn emit(&mut self, purpose: IoPurpose, kind: SdioRequestKind) -> AicAction {
    let id = self.io.next_request_id;
    self.io.next_request_id = self.io.next_request_id.wrapping_add(1).max(1);
    self.io.pending = Some(PendingIo { id, purpose });   // 发出即占用唯一的飞行槽
    AicAction::SubmitSdio(SdioRequest { id, kind })
}
```

`PendingIo` 里的 `purpose` 是回执分发的钥匙：同一个 `ReadByte` 请求，在 mailbox 流程里叫 `MailboxFlow`，在 RX 排水里叫 `ReceiveCount`，在 TX 里叫 `TransmitFlow`。完成回来时，`consume_completion` 按 `purpose` 把结果交给对应的消费函数（`consume_mailbox_response` / `consume_receive_count` / `consume_transmit_flow`……）。**请求本身不带上下文，上下文跟着飞行槽走。**

## 2.5 fail：失败是"有秩序地收场"

任何不可恢复的错误都汇入 `progress.rs:206` 的 `fail`：

```rust
pub(super) fn fail(&mut self, error: AicError) -> AicAction {
    self.lifecycle.state = AicState::Failed;
    self.io.pending = None;          // 丢弃飞行槽（外界会收到错误并中止硬件侧）
    self.io.next = None;
    self.lifecycle.mailbox = None;
    self.lifecycle.control = None;
    if let Some(active) = self.data.active_tx.take() {
        // 正在发送的帧：把 token 还回去
        self.data.events.push_back(AicEvent::TransmitComplete(active.token));
    }
    // 队列里所有未发送的帧：token 全部归还
    let tokens: Vec<_> = self.data.tx.drain_tokens().collect();
    self.data.events.extend(tokens.into_iter().map(AicEvent::TransmitComplete));
    self.data.events.push_back(AicEvent::Failed(error));
    AicAction::Event(self.data.events.pop_front().unwrap())
}
```

读这段代码的顺序：清掉所有子状态机 → **把持有的一切所有权物归原主**（正在发的那一帧的 token、队列里所有帧的 token）→ 发布 `Failed` 事件。之后进入 `Failed` 状态，`advance` 返回 `Idle`——设备静止，但没有任何 buffer 丢失。外界（适配层）收到一串 `TransmitComplete` 后收到 `Failed`，知道所有发包都终止了。

## 2.6 小结

- 一次 `advance` = 按固定顺序检查 8 个条件，第一个有结论的条件决定返回值。
- 状态机自己只做决策和记账；所有副作用（执行 SDIO、等待、定时）都委托给外界。
- 单飞行请求 + purpose 分发表 = 一条 SDIO 总线上的严格串行化，取消和错误处理因此变得简单。
- 失败路径的第一义务是归还所有权（token），然后才是报告错误。

下一章看最常用的子状态机：mailbox（固件命令）。
