# 第 1 章 驱动为什么不能再自己等

> 对照代码：`drivers/net/aic8800/src/device/model.rs`（全部类型）、`progress.rs`（advance 主体）、`drivers/net/aic8800/Cargo.toml`（依赖清单）
> 对比旧代码：重构前的 `src/fdrv/protocol/cmd.rs` 的 `send_cmd`（`git show 0fc626fa4^:drivers/net/aic8800/src/fdrv/protocol/cmd.rs`）

## 1.1 旧驱动是怎么"等"的

重构前，驱动发一条固件命令的典型写法是：

```rust
// 旧代码（已删除）
fn send_cmd(...) -> Result<Vec<u8>, WifiError> {
    // 写 FIFO
    transport.write_fifo(...)?;
    // 循环等待固件回 CFM：读状态寄存器，没到就 yield 让出 CPU，直到超时
    for _ in 0..max_retries {
        if let Some(rsp) = transport.poll_rsp() { return Ok(rsp); }
        crate::runtime::runtime().yield_now();
    }
    Err(WifiError::Timeout)
}
```

这段代码隐藏着三个问题：

**问题一：怎么等，驱动不知道。**
"等"有好几种等法：等中断、等一个很短的寄存器翻转（值得空转）、等一个绝对时刻。哪种等法合适，取决于运行环境——中断是注册在哪里的、CPU 上还有没有别的活、定时器精度多少。这些是**网络运行时**的知识。驱动在自己内部写死 `yield_now` 循环，等于替运行时做了它不该做的决定。驱动在等命令回应期间占着 CPU 反复让出，运行时却不知道这中间还有没有别的事可做。

**问题二：等待期间，别人碰不了这个驱动。**
旧 `send_cmd` 一旦进入等待循环，外面的控制面（比如"取消这个操作"、"设备要关闭"）就没有干净的入口插入。要打断它，只能靠超时自然结束，或者靠一个跨线程的信号量——而驱动自己开线程正是旧模型被否定的原因之一。

**问题三：等待本身不可测试。**
`yield_now`、`sleep_ms` 都依赖真实操作系统的调度。要测"超时路径"，就必须真的等上几秒。状态机测试（`progress.rs` 末尾那一组 `#[test]`）能喂假时间立刻验证任意路径，就是因为新模型把等待变成了数据而不是代码。

## 1.2 新模型：把"等待"变成"报告"

新核心的入口只有一个（`progress.rs:21`）：

```rust
pub fn advance(&mut self, input: AicInput) -> AicAction {
```

它接收一次输入，推进一步，返回一个 action。**它自己永远不会等**。所有"需要等"的情形，都被翻译成两种输出（`model.rs:204`）：

```rust
pub enum AicAction {
    SubmitSdio(SdioRequest),          // 需要外界执行一个 SDIO 操作，完成后把结果喂回来
    AbortSdio { request_id: u64 },    // 需要外界中止正在执行的那个 SDIO 操作
    RetryAt(MonotonicTime),           // 现在没事可做，请在某时刻再调用我一次
    WaitForInterrupt,                 // 现在没事可做，请在有中断（或事件）时再调用我
    Event(AicEvent),                  // 我产出了一个对外可见的事件（收到帧、命令完成……）
    Idle,                             // 什么都不用做
}
```

注意 `RetryAt` 和 `WaitForInterrupt` 的本质：它们不是"我在等"，而是"我告诉你该等什么"。真正去等的是外面的 owner。

对应地，时间也变成了输入（`model.rs:10`）：

```rust
/// Absolute value in the monotonic-clock domain supplied by the owner.
pub struct MonotonicTime(u64);
```

驱动的核心**没有自己的时钟**。`now` 永远是外界喂进来的。"两秒后重试"被表达为 `now.after(Duration::from_millis(2))` 算出的一个绝对时刻，然后报 `RetryAt(那个时刻)`。外界（ax-net 运行时）决定用定时器还是用别的手段在那个时刻叫醒。

Cargo.toml 把这个原则钉死在依赖上：

```toml
[dependencies]
dma-api = { workspace = true, optional = true }
log = { workspace = true }
rdif-eth = { workspace = true, optional = true }
ringbuf = { workspace = true, optional = true }
sdmmc-host = { workspace = true, optional = true }
sdmmc-protocol = { workspace = true, ... }
thiserror = { workspace = true }
tock-registers = { workspace = true }
```

没有 `ax-sync`、没有 `ax-task`、没有 `axruntime`。设计文档 §依赖与源码门禁 明文规定：

> `aic8800` 默认依赖树不得出现 `rd-net`、`rdif-eth`、`ax-sync`、`ax-task` 或 `axruntime`；不得出现全局 runtime、线程创建、sleep、yield 或 spawn。

一个没有锁、没有线程、没有休眠原语的模块，在物理上不可能"自己等"。

## 1.3 这带来了什么能力

**能力一：取消可以在任何一步生效。**
既然每一步推进都经过 `advance`，取消就是一个输入事件。`progress.rs:68` 的 `consume_input` 收到 `ControlRequest::Cancel` 后置一个标志；下一次 `advance` 检测到标志，就对外报 `AbortSdio { request_id }`——精确中止**正在飞的那个** SDIO 请求（`progress.rs:176` 的 `drive_shutdown`、`progress.rs:128` 的 `consume_completion` 都有对应处理）。旧模型里"等待循环中被取消"根本无处安放，这里只是一个分支。

**能力二：超时是两级结构，各管各的。**
核心里的状态机（mailbox）有自己的绝对 deadline（`mailbox.rs:10` `MAILBOX_TIMEOUT`），到点就报错；调用方（`WifiControl` 适配层）还有一层更大的 deadline（`AicRdifOptions::control_timeout`）。因为每一步推进都带时间戳，两层 deadline 都是"比较两个数字"，不需要任何定时器对象。

**能力三：每一步都短，且可验证。**
`advance` 一次只做一个决策、最多发出一个 SDIO 请求。单步的短小使得它的行为可以被穷举式测试：`progress.rs` 末尾的测试用假时间喂 `advance`，断言返回的 action 序列（例如 `startup_begins_with_protocol_owned_function_lifecycle` 断言启动的第一、第二步分别是 EnableFunction、SetBlockSize）。

## 1.4 一个完整的"等待"实例：流控信用不足

看完这一章，先拿一个最小的实例验证一下理解。`data_plane.rs:92` 的 `consume_transmit_flow`：

```rust
pub(super) fn consume_transmit_flow(&mut self, response: SdioResponse, now: MonotonicTime)
    -> Result<(), AicError> {
    let credits = flow_credits(expect_byte(response)?);
    // ...
    if credits == 0 || usize::from(credits) * BLOCK_SIZE <= active.wire_frame.len() {
        self.lifecycle.retry_at = Some(now.after(IO_RETRY));   // IO_RETRY = 1ms
        return Ok(());
    }
    self.io.next = Some((IoPurpose::TransmitData, write_fifo(...)));
    Ok(())
}
```

固件的发送 FIFO 满了（信用为 0）时，旧驱动会在这里 `yield_now` 循环。新驱动做的事只有一件：记下"1 毫秒后叫我"。之后 `advance` 会先检查这个时刻（`progress.rs` 里 `retry_at` 的判断），没到就报 `RetryAt` 让外界去等。**等多久、怎么等，都是运行时的决定；驱动只陈述事实：现在发不出去，1 毫秒后再试。**

这就是本章要建立的最重要认知：**状态机的输出不是"结果"，是"下一步需要什么"；等待不是行为，是数据。** 后面所有章节（mailbox、startup、TX、控制命令）都是这个原则的具体展开。
