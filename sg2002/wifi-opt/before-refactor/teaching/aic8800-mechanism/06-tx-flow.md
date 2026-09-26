# 第 6 章 发包与 buffer 流转

> 对照代码：
> - `drivers/net/aic8800/src/rdif/device/queues.rs`（四道环的构造与队列实现）
> - `drivers/net/aic8800/src/rdif/owner/output.rs`（token 记账与 buffer 归还）
> - `drivers/net/aic8800/src/device/tx.rs`（核心的 TX 队列与组帧）
> - `drivers/net/aic8800/src/device/data_plane.rs`（TX 推进：信用检查与写 FIFO）
> - `protocol.rs` 的 `ethernet_tx_frame`（组帧）

## 6.1 一帧数据要走的路

协议层（网络栈）要发一帧以太网数据，最终目标是 AIC 芯片的写 FIFO。中间要过四道 SPSC 环（`queues.rs` 的 `queue_parts`）：

```text
协议层                         适配层 owner                    核心
  │                                                                 │
  │  ① tx_submit 环（协议层 → owner）                                │
  │     内容：DmaBuffer（协议层写好数据的 buffer）                     │
  ├──────────────▶ owner 取出，拷贝出帧数据，留下 token 记账            │
  │                                                                 │
  │  ② 核心 Tx 事件：帧进核心 tx 队列（容量 128，带 token）             │
  │                                                                 │
  │  ③ 核心推进：信用检查 → 写 WR_FIFO → 固件取走                     │
  │                                                                 │
  │  ④ 核心报 TransmitComplete(token)                               │
  │                                                                 │
  │  ⑤ owner 按 token 找到 buffer，推进 tx_complete 环                │
  │     内容：DmaBuffer（发完的 buffer）                               │
  ◀──────────────┤ 协议层 reclaim 取回，重新利用                       │
```

先看①和⑤这两道环为什么存在，再看②③④核心内部怎么推进。

## 6.2 为什么 buffer 必须跨环流转

协议层的 DmaBuffer 不是"一段随便用的内存"，它是**有数量的**。`into_parts` 时按 `QueueConfig.ring_size`（默认 32，`rdif/device/endpoints/device.rs`）建的池，每个 DmaBuffer 代表"一次在飞的发送资格"：

- 协议层手里没有 buffer，就发不了下一帧——这是**结构性流控**：在飞帧数的上限 = 池大小，不用任何计数器协商；
- buffer 在谁手里，谁就对那段内存负责。发出去的帧如果被丢弃（断网清队列、设备失败），buffer 必须回到协议层，否则池慢慢空掉，协议层永远卡死。

所以协议层的 `AicTxQueue::submit` 只是把 buffer 推进①号环（`queues.rs`）：

```rust
fn submit(&mut self, buffer: DmaBuffer) -> Result<(), SubmitError> {
    if buffer.capacity() < self.config.buf_size {
        return Err(SubmitError::new(buffer, NetError::InvalidParts));  // 环满：原样退回
    }
    self.submit.try_push(buffer)
        .map_err(|buffer| SubmitError::new(buffer, NetError::Retry))
}
```

环满（owner 还没取走上一个）就原样退回 buffer。**退回去 = 协议层保留资格，稍后重试；吞掉 = 资格消失。** 这就是 buffer 与普通内存不同的地方：它是资格本身。

## 6.3 owner 取帧：token 记账

owner 从①号环取出 buffer 后（`output.rs` 的 `take_tx_frame`）：

```rust
let buffer = self.queues.tx_submit.try_pop()?;
let length = buffer.len();
buffer.complete_for_cpu(length);
let frame = buffer.read_with_cpu(length, |bytes| bytes.to_vec());  // 拷贝出帧数据
let token = TxToken::new(self.next_tx_token);
self.next_tx_token = self.next_tx_token.wrapping_add(1).max(1);
self.tx_tokens.push_back((token, buffer));      // 记账：token ↔ buffer
Some((token, frame))
```

这里发生三件事：把帧数据**拷贝**出来（SDIO 传输要一块连续内存，且核心不依赖 DmaBuffer 类型）；给这次发送分配一个递增的 `TxToken`（`model.rs:46`，就是一个 u64 新类型）；把 (token, buffer) 记进 `tx_tokens` 队列。

**为什么核心只认识 TxToken 而不是 DmaBuffer**：核心是零依赖的纯状态机（第 1 章的依赖清单），它不能引用 `dma-api` 的 buffer 类型——那是适配层与运行时之间的概念。token 是核心与适配层之间关于"这一帧"的唯一共同语言。发完时核心报 `TransmitComplete(token)`，适配层按 token 查账找到 buffer 还回⑤号环（`output.rs` 的 `publish_tx_completion`：在 `tx_tokens` 里按 token 定位、移除、push 进 tx_complete 环）。查不到 token（`CompletionMismatch`）就是真 bug，直接报错。

## 6.4 核心内部：一帧在飞的三个步骤

核心收到 `AicInputEvent::Tx { token, frame }` 只是把帧放进容量 128 的 `TxState` 队列（`tx.rs`）。真正发送在 `drive_ready` 里推进（`data_plane.rs`）：

```text
prepare_next_transmit（data_plane.rs:127）
  取队头帧 → ethernet_tx_frame 组帧（加 host descriptor，见 6.5）
  → 放进 active_tx（同一时刻只有一帧在飞，对应单飞行规则）

下一轮 advance：
  TransmitFlow（data_plane.rs:92）  读 flow_control 寄存器
    信用不够 → retry_at = now + 1ms（IO_RETRY），改天再试
    信用够   → 排 TransmitData
  TransmitData（data_plane.rs:114） 写 WR_FIFO（整帧，CMD53）
    完成     → 报 TransmitComplete(token)，清空 active_tx
```

信用检查的精确条件（`data_plane.rs:92`）：

```rust
let credits = flow_credits(expect_byte(response)?);
if credits == 0 || usize::from(credits) * BLOCK_SIZE <= active.wire_frame.len() {
    self.lifecycle.retry_at = Some(now.after(IO_RETRY));
    return Ok(());
}
```

流控寄存器的低 7 位是**固件写 FIFO 还能接受的 512 字节块数**。判据是"信用块数 × 512 必须**严格大于**帧长"（等于不够：固件要求有空余）。信用不足时的处理就是第 1 章那个例子：记 `retry_at`，报 `RetryAt`，让外界等 1ms——**这是核心里唯一残留的"轮询"**，但它的形态是数据而非循环，owner 在等待期间可以处理别的事（比如 RX 排水），1ms 后回来再试。

信用什么时候恢复？固件消费写 FIFO 之后。固件消费了数据会拉 CARD_INT（第 5 章），于是 owner 被唤醒，新一轮 advance 里 `prepare_next_transmit` 和 `TransmitFlow` 自然被再走一遍。

## 6.5 组帧：以太网帧变固件帧

`protocol.rs` 的 `ethernet_tx_frame` 把以太网帧包成固件认识的格式：SDIO 4 字节头（长度 + `SDIO_TYPE_DATA` 类型 + CRC8）+ 28 字节 host descriptor（`HOST_DESCRIPTOR_SIZE`，含 vif/sta 索引、长度等）+ 帧体，然后整体按 4 字节对齐、补尾、按 512 字节块对齐。组帧失败（比如帧太大）时 token 直接退回（`tx.rs` 的 `take_wire_frame` 返回 `Err(token)`，`prepare_next_transmit` 把 token 变成 `TransmitComplete` 事件还回去）——**任何一条路径都不吞 token**。

## 6.6 失败与关闭：token 一个都不能少

回顾第 2 章的 `fail`：正在飞的帧（`active_tx`）和队列里所有帧（`TxState`）的 token 全部以 `TransmitComplete` 事件归还。关闭流程 `drive_shutdown`（`progress.rs:176`）同样先 `drain_tokens` 归还所有排队帧的 token，再发关闭用的 SDIO 写。

加上 6.3 的"查不到 token 就报错"，整个 TX 路径的账目闭合规则是：**帧进核心时 token 进账，帧出核心时（发送完成、丢弃、失败、关闭）token 必须原样出账。** 出账的四种方式覆盖了所有路径，协议层的池永远不会因驱动侧原因缩水。

## 6.7 环满时发生了什么（背压的三级传导）

把四道环连起来看"协议层发得太快"时压力怎么传导：

1. 协议层 push ①号环 → 环满（owner 没取）→ `try_push` 失败 → buffer 退回协议层 → 协议层看到 submit 失败，**自己暂停**（ax-net 的 TX 背压语义：等 group 空间 generation）。
2. owner 取走了帧，但核心 tx 队列满（128）→ `TxQueueFull` 错误 → 设备 fail？看代码：`consume_input` 里 `map_err(|_| AicError::TxQueueFull)` 会走 `fail`。等等——这里值得注意：核心的 tx 队列满被视为错误而非背压。为什么？因为①号环的容量（默认 32）远小于 128，正常情况下核心队列不会先满；真满了说明某个上游环节的背压失效了，按故障处理更安全。
3. owner 发完帧，⑤号环满（协议层没 reclaim）→ `publish_tx_completion` 把 buffer 扣在 `pending_tx_completion` 单槽，owner 报 WaitForInterrupt；协议层 reclaim 后精确唤醒 group（第 8 章的 `schedule_task`），owner 的 `flush()` 把扣住的 buffer 送进环。

第 1 和第 3 级的共同模式：**"满"永远表现为"上游停下来等"，并且等待者有一个明确的唤醒者。** 没有人自旋，也没有人丢数据。

## 6.8 小结

- 发一帧 = 跨四道环 + 核心三步（组帧、信用检查、写 FIFO），token 全程记账。
- buffer 是资格不是普通内存：任何路径（满、失败、关闭、丢弃）都必须把它送回协议层。
- 核心与适配层之间用 TxToken 交流，核心不依赖任何 OS 类型。
- 信用不足是核心里唯一的轮询点，形态是 `RetryAt` 数据，由固件消费 FIFO 后拉 CARD_INT 自然解套。
- 背压的三级传导全部是"停 + 被下游唤醒"，无自旋、无丢失。
