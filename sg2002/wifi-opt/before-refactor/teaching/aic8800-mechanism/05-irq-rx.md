# 第 5 章 中断与收包

> 对照代码：
> - `drivers/blk/sdmmc-host/src/irq/mod.rs`（HostParts 拆分）、`drivers/blk/sdhci-host/src/lib.rs`（SdhciIrqHandle/SdhciCardIrqHandle、CARD_INT mask/rearm）
> - `drivers/net/aic8800/src/rdif/device/shared.rs`（IrqLatch）
> - `drivers/net/aic8800/src/rdif/device/endpoints/irq.rs`（hard endpoint）
> - `drivers/net/aic8800/src/rdif/owner/progress.rs`（合并事件推进）
> - `drivers/net/aic8800/src/device/data_plane.rs`（RX 排水）、`rx.rs`（FIFO 解析）

## 5.1 一条物理线上挂着两种中断

重构后，SDHCI 控制器上报到 CPU 的中断有两类，都走同一条物理 IRQ 线：

1. **CARD_INT**——卡要通信。AIC8800 芯片把一条电平信号拉高并保持，直到它的 FIFO 被排空。这是电平信号：信号不会"再来一次"，不处理掉它就一直挂着。
2. **事务完成中断**——CMD_COMPLETE、XFER_COMPLETE、BUF_READY、错误位。这些是 SDHCI 控制器执行 CMD52/CMD53 的中间状态，现在也路由到 CPU 中断线（`sdhci-host` 的 `enable_completion_irq` 打开它们）。

旧设计里第 2 类靠同步轮询（PIO 循环读状态寄存器），新设计里它们**也走中断**：一次物理中断到达时，可能同时携带"卡有话说"和"刚才那个 CMD53 完成了"两个事实。5.3 会讲这两个事实怎么同时被消费。

硬中断端点如何拿到这两种事实：`SdMmcIrqHost::into_parts()` 把主机一次性拆成三个 move-only 部分（`sdmmc-host/src/irq/mod.rs`）：

```rust
pub struct HostParts<B, I, C> {
    pub bus: B,           // 总线和事务能力 → 留在 task 上下文（owner）
    pub irq: I,           // 硬中断端点 → 注册进 IRQ 框架
    pub card_irq: Option<C>, // CARD_INT 的 mask/rearm 控制 → 也归 owner
}
```

硬中断端点 `SdhciIrqHandle::handle_irq()` 在中断上下文做三件事（`sdhci-host/src/lib.rs` 的 `handle_irq_core`）：读状态寄存器；把**非 CARD_INT 的完成位**先缓存进代际 mailbox（`cache_if_current`，只有与当前活动事务同代才生效）再 W1C 清除；CARD_INT 位**不清**、只把它的信号掩掉（mask）。缓存进代际 mailbox 的完成位稍后由 task 上下文按需取走（`take_normal`）。区分这两种处置的原因：完成位属于"发生过的一次事务结束"，确认掉就没事了；CARD_INT 是卡维持的电平（第 5.1 节），清除会丢失"卡还有话说"的事实，只能 mask。所有进一步的动作（取缓存、排空 FIFO）都在 task 上下文。

## 5.2 IrqLatch：中断到 owner 之间的一个原子字

硬中断端点拿到 `Event` 后，怎么把它交给 task 上下文（owner）？中断上下文不能分配内存、不能拿锁、不能做任何可能阻塞的事。所以通道是一个**预先分配好的单原子变量**（`rdif/device/shared.rs:17`）：

```rust
const IRQ_CARD: u8 = 1 << 0;       // 卡要通信
const IRQ_TRANSFER: u8 = 1 << 1;   // 某个事务完成了
const IRQ_ERROR: u8 = 1 << 2;      // 总线错误
const IRQ_FLAG_BITS: u32 = 3;
const IRQ_FLAG_MASK: u64 = 0b111;  // 低 3 位 = 标志
// 高 61 位 = 递增序列号

pub(crate) struct IrqLatch { state: AtomicU64 }
```

**publish**（中断上下文调用）：把事件的标志 OR 进低 3 位，序列号 +1，用 CAS 写回。**take**（owner 调用）：CAS 清掉低 3 位（保留序列号），返回快照。`IrqSnapshot` 的结构（`model.rs`）：

```rust
pub struct IrqSnapshot {
    pub sequence: u64,            // 拿走的是第几版事实
    pub card_interrupt: bool,
    pub transfer_complete: bool,
    pub error: Option<SdioFailure>,
}
```

这个设计要解决的真实问题是**合流与次序**：

- 合流：中断连续来了两次（一次 CARD_INT、一次事务完成），owner 还没来取。两个事件被 OR 进同一个字，owner 一次取走一个**合体快照**，两个事实都不丢。
- 次序：take 清标志但保留序列号。owner 之后拿序列号判断"这个快照比上一个新吗"（`consume_input` 里 `snapshot.sequence > last_irq_sequence` 的判断，见 `progress.rs`），迟到的旧快照被丢弃。
- 取走瞬间的新事件：take 的 CAS 只清标志位，如果中断恰好在 CAS 前又 publish 了一次，CAS 会失败重试，拿到合并后的新值——`shared.rs` 末尾的测试 `irq_published_while_snapshot_is_taken_is_never_hidden_by_the_same_sequence` 专门验证这一点。

**为什么不丢**：publish 只做 OR，任何时刻 publish 的事实都会留在字里直到被 take；take 只清"已经转成快照"的位，未读的新事实保留。

## 5.3 一个快照同时推两件事

这是本次重构最精妙的一处，在 `rdif/owner/progress.rs:97` 的 `advance_with_rearm`：

```rust
let snapshot = self.irq_latch.take();
let cause = if snapshot.is_some() {
    ProgressCause::AcknowledgedIrq      // 事务推进用"已确认中断"作为原因
} else {
    ProgressCause::RegisterRetry        // 没有中断就只做寄存器重试
};
if let Some(snapshot) = snapshot && self.started {
    let action = self.device.advance(AicInput {
        now: ...,
        event: Some(AicInputEvent::Irq(snapshot)),   // ① 快照喂给核心：记 irq_pending
    });
    if let Some(progress) = self.consume_action(action, now_nanos)? {
        // 注释原文：A controller may latch CARD_INT together with command/data
        // completion. Preserve the card fact in the core, then still advance
        // the active host request with this same acknowledged snapshot so
        // neither half of the combined event is lost.
        if self.active.is_none() {
            return self.finish_progress(progress, rearm_after_step);
        }
    }
}
self.advance_with_cause(now_nanos, cause, rearm_after_step);  // ② 同一快照再推活动事务
```

场景：一个 CMD53 读 FIFO 正在飞行，此时卡又拉高了 CARD_INT。控制器把"XFER_COMPLETE + CARD_INT"一次性锁存。快照同时携带 `transfer_complete` 和 `card_interrupt`：

- ① 把快照喂给核心：`card_interrupt` 置起 `io.irq_pending`（第 2 章的事件落袋）；
- ② 同一个快照作为 `ProgressCause::AcknowledgedIrq` 传给 `advance_with_cause`，把正在飞的 CMD53 推到完成。

如果只做①，CMD53 的完成就丢了（没人再推进它）；只做②，卡的新数据就丢了。**"一个事件、两个消费者"靠同一次 take 保证两半都不丢。**

## 5.4 RX 排水：从"有中断"到"有帧"

`data_plane.rs:14` 的 `drive_ready` 里 RX 的入口只有一行：

```rust
if self.io.irq_pending {
    self.io.irq_pending = false;
    return self.emit(IoPurpose::ReceiveCount, read_byte(1, self.registers.block_count));
}
```

后续在 `consume_receive_count`（`data_plane.rs:48`）：

```rust
let Some(blocks) = interrupt_block_count(expect_byte(response)?) else {
    self.io.irq_pending = true;    // OTHER 位置位：是软件中断，稍后再处理
    return Ok(());
};
if blocks != 0 {
    self.io.next = Some((IoPurpose::ReceiveData,
        read_fifo(1, self.registers.read_fifo, usize::from(blocks) * BLOCK_SIZE)));
}
```

`interrupt_block_count`（`registers.rs`）先看第 7 位：OTHER 置位时块数不可信，返回 None——此时把 `irq_pending` 重新置上，下一轮再处理（OTHER 是固件的软件中断通知，处理它需要别的方式，不能拿它当块数读数据）。块数为 0 说明卡只是清了中断但没数据（或者数据在别处），安静结束。块数非 0 就排一个 `ReceiveData` 请求：按块数 × 512 字节读 FIFO。

`consume_receive_data`（`data_plane.rs:66`）拿到整块数据后交给 `rx.rs` 的 `parse_fifo`。

## 5.5 FIFO 聚合帧的格式与解析

固件不是一帧一帧往 FIFO 里放的，而是把多帧**聚合**成一整块，每帧一个 4 字节头：

```text
[0..2]  本帧长度（LE u16）
[2]     帧类型（bit6=1 表示 CFG 帧；值 = SDIO_TYPE_DATA 的是数据帧）
[3]     保留
后面跟帧体；每帧整体 4 字节对齐
```

`rx.rs` 的 `parse_fifo` 按这个格式逐帧切分：

- **DATA 帧**（`packet_type == SDIO_TYPE_DATA`）：帧体前 60 字节是厂商硬件头（`HARDWARE_HEADER`），真正的 802.11 MPDU 在 60 字节之后。解析器剥掉硬件头，把 MPDU 原样放进 `ParsedFrame::Data`。长度检查（`packet_len < 24` 或越界）直接 `break`——**宁可少收不可乱读**，越界说明固件给的数据不自洽。
- **CFG 帧**（bit6 置位）：帧体是固件消息。取偏移 0 的消息号，**奇数号是回应（Confirmation）、偶数号是指示（Indication）**（`message_id & 1 == 1` 的分支）。载荷从偏移 8 或 12 开始（`declared` 长度字段在 [6..8]）。

数据帧的去向（`data_plane.rs:66` 的 `consume_receive_data`）：

```rust
ParsedFrame::Data(frame) => {
    if self.data.rx.push(frame.clone()) {
        self.data.events.push_back(AicEvent::Receive(frame));
    }
}
```

`RxState` 是容量 256 的队列（`rx.rs` 的 `RX_CAPACITY`）。**队满时 `push` 返回 false，帧被静默丢弃、不发 `Receive` 事件**。这是核心层唯一的有界缓冲：溢出时丢帧是明确的设计选择——核心不能无界分配内存，而协议栈（TCP 等）本身就要处理丢包。CFG 帧在 RX 排水路径里只打日志；命令回应走 mailbox 自己的 Read（第 3 章），两条路径读同一个 FIFO，靠单飞行规则串行。

## 5.6 帧如何到协议层：适配层再跨一道环

核心把 `AicEvent::Receive(frame)` 交给适配层。适配层的 `publish_rx`（`rdif/owner/output.rs`）要做的事：**把 Vec 拷进协议层的 DmaBuffer，通过 rx_complete 环交给协议栈**。这里又有一道背压：

```rust
fn publish_rx(&mut self, frame: Vec<u8>) -> Result<bool, AicRdifError> {
    if frame.len() > self.queues.rx_frame_size {
        return Err(AicError::MalformedResponse.into());   // 帧超过配置上限：故障，不硬塞
    }
    if self.pending_rx_frame.is_some() || self.pending_rx_completion.is_some() {
        return Ok(false);                                  // 上一帧还没送走：先扣住
    }
    let Some(mut buffer) = self.queues.rx_submit.try_pop() else {
        self.pending_rx_frame = Some(frame);               // 没有空 buffer：扣住帧
        return Ok(false);
    };
    // 有 buffer：拷贝，推进 rx_complete 环
    let completion = RxCompletion { buffer, packet_len: frame.len() };
    match self.queues.rx_complete.try_push(completion) {
        Ok(()) => Ok(true),
        Err(completion) => { self.pending_rx_completion = Some(completion); Ok(false) }
    }
}
```

三种情况：
1. `rx_submit` 环（协议层放进来的空 buffer）有货、`rx_complete` 环（装好数据的 buffer 回协议层）有空间 → 正常流转；
2. 没有空 buffer（协议层还没把用过的 buffer 还回来）→ 帧扣在 `pending_rx_frame` 单槽里，等协议层来 buffer；
3. `rx_complete` 满（协议层还没消费上一帧）→ 完成的帧扣在 `pending_rx_completion` 单槽里。

扣住之后，owner 报 `WaitForInterrupt`——但此时没有中断会来，谁来解扣？答案是协议层还 buffer 时会精确调度这个 group（第 8 章的 `schedule_task`），于是 owner 被再次唤醒、`flush()` 把扣住的帧送走。**每一环的"满"都变成"暂停 + 被下游唤醒"，而不是丢数据或阻塞。**

## 5.7 小结

- 中断上下文只做一件事：读状态 → 完成位缓存后确认、CARD_INT 只 mask → 把事实 OR 进 IrqLatch → 返回快照。所有消费都在 task 上下文。
- IrqLatch 用一个原子字的"标志位 + 序列号"同时解决合流、次序、取走瞬间竞争三个问题。
- 合并事件（CARD_INT + 事务完成）用同一个快照推两个消费者，两半都不丢。
- RX 路径是三级流水：块数 → FIFO 数据 → 聚合帧解析 → 有界队列 → 适配层跨环拷贝。每一级的"满"都有明确的处置（丢帧或扣留+等唤醒），没有任何一步无限等待。
