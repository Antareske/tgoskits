# 第 3 章 固件命令怎么发出去：mailbox 状态机

> 对照代码：`drivers/net/aic8800/src/device/mailbox.rs`（状态机主体）、`protocol.rs`（帧格式）、`registers.rs`（寄存器语义）

## 3.1 先说硬件事实

AIC8800 芯片上跑着一块固件。固件通过 SDIO 与主 CPU 通信，通道是两个 FIFO：

- **写 FIFO**（寄存器 `write_fifo`，D80 型号地址 0x10）：主 CPU 用 CMD53 把一帧数据写进去，固件读取并执行。发命令就是写这个 FIFO。
- **读 FIFO**（寄存器 `read_fifo`，D80 型号地址 0x0f）：固件把回应帧放进这里，主 CPU 用 CMD53 读出来。

还有两个控制寄存器约束着这两条通道：

- **流控寄存器**（`flow_control`，0x03）：低 7 位是**信用数**——固件写 FIFO 还能接受的 512 字节块的数量。信用为 0 表示固件缓冲区满了；旧驱动的检查函数在信用为 0（或不足以容纳待发数据）时一律返回"不可用"、不执行写（重构前 `fdrv/core/sdio_transport.rs` 的 `check_flow_ctrl_for_size`）。所以写之前必须先读信用。
- **状态寄存器**（`block_count`，0x04）：低 7 位是**读 FIFO 里待读的数据块数**；第 7 位（OTHER）是"软件中断"标志，表示有别的通知，此时块数不可信。读回应之前先看它：块数为 0 就说明固件还没回。

要执行一个命令，完整过程是：**查信用 → 写命令帧到写 FIFO → 等固件处理 → 轮询块数 → 读回应帧 → 检查回应里的消息号与请求匹配**。这就是 mailbox 状态机的全部内容。

命令帧的格式由 `protocol.rs` 的 `command_frame` 构造：4 字节 SDIO 头（长度 + 类型 `SDIO_TYPE_CFG_CMD_RSP` + V3 型号下 1 字节 CRC8），**4 字节哑字**（`DUMMY_WORD_SIZE`），8 字节 LMAC 头（消息号、目标任务号、驱动任务号、载荷长度），后面跟载荷。回应帧**没有哑字**：SDIO 头之后直接是 LMAC 头（`confirmation_payload` 在原始 FIFO 数据的偏移 4 读消息号、偏移 10 读声明长度、偏移 16 起是载荷），消息号 = 请求消息号 + 1。

## 3.2 状态机：六个阶段

`mailbox.rs:16`：

```rust
enum MailboxPhase {
    Flow,              // 读流控信用，够才往下走
    Write,             // 把命令帧写进写 FIFO
    Settle,            // 写完等 2ms（固件把回应入队需要时间）
    Count,             // 轮询读 FIFO 的块数
    Read { length },   // 把回应帧读出来
    Complete,          // 匹配消息号，交给上层
}
```

一次 mailbox 运转的完整时序（`drive_mailbox`，`mailbox.rs:36`）：

```text
Flow   → 发出 ReadByte(flow_control)           [MailboxFlow]
        → 回应：credits == 0 ？记 retry_at=+1ms 回到 Flow；否则进 Write
Write  → 发出 Write(WR_FIFO, 整帧)             [MailboxWrite]
        → 回应成功：进 Settle，记 retry_at=+2ms
Settle → 时刻到了：进 Count
Count  → 发出 ReadByte(block_count)            [MailboxCount]
        → 回应：块数 0？记 retry_at=+1ms 重来；否则进 Read{块数*512}
Read   → 发出 Read(RD_FIFO, length)            [MailboxRead]
        → 回应成功：用 expected_message_id 匹配、取载荷，进 Complete
Complete → 交给消费方（启动流程或控制命令队列），然后回到主状态机
```

每个阶段只产生一个 SDIO 请求——这正是第 2 章"单飞行"规则下的自然形态：mailbox 把一次逻辑上的"发命令"拆成了 5 个串行 SDIO 操作，每个操作之间用 `retry_at` 或状态转移衔接。

## 3.3 三个时间常数的道理

`mailbox.rs:10`：

```rust
const MAILBOX_TIMEOUT: Duration = Duration::from_secs(5);   // 整条命令的 deadline
const MAILBOX_FLOW_RETRY: Duration = Duration::from_millis(1); // 信用不足/无回应的轮询间隔
const MAILBOX_SETTLE: Duration = Duration::from_millis(2);  // 写完到可读之间的静置
const MAX_MAILBOX_FLOW_RETRIES: u16 = 100;                  // 信用轮询次数上限
```

- **Settle 2ms**：命令写进 FIFO 后，固件要"解析 → 执行 → 把回应写进读 FIFO"才能让块数变成非 0。立刻读块数几乎必然读到 0，于是白白轮询。2ms 静置让第一次 Count 就有大概率命中。
- **Flow/Count 的 1ms 间隔**：信用不足或回应未到，本质是"固件还没腾出手"。1ms 是轮询的粒度——它被表达为 `retry_at`（第 1 章的例子），由外界决定怎么等。
- **5s deadline + 100 次上限**：双保险。信用连续 100 次为 0（约 100ms+）或整条命令超过 5 秒，都判 `MailboxTimeout`。上限的存在保证命令不会因为固件死掉而永远占着 mailbox。

## 3.4 匹配回应：为什么必须检查消息号

`confirmation_payload`（`protocol.rs`）做两件事：取帧内偏移 4 的消息号，与 `expected_message_id` 比较；不等就报错。

这个检查防的是**错位**：读 FIFO 里可能有别的帧（异步的通知帧、上一次命令迟到的回应）。如果只按"块数非 0 就读"，可能读到别人的帧并当成自己的回应，于是命令内容和结果对不上。消息号匹配保证"读到的确实是这个命令的回应"。回应帧的消息号 = 请求消息号 + 1，这是固件协议约定（`begin_lmac_mailbox` 里 `expected_message_id: message_id + 1`，见 `mailbox.rs:178`）。

同样地，读 FIFO 里的帧还可能是不请自来的**指示帧**（固件主动上报状态变化）。`rx.rs` 的 `parse_fifo` 按消息号的奇偶把它们和回应帧分开（第 5 章细讲）。

## 3.5 两种 mailbox：debug 与 LMAC

芯片运行有两个阶段，各自有一条命令通道：

- **bootrom 阶段**（启动早期、应用固件还没跑起来）：`begin_debug_mailbox`（`mailbox.rs:161`）发 debug 命令，目标任务号是 `TASK_DBG`，消息号以 `0x04xx` 开头（`DBG_MEM_READ_REQ`、`DBG_MEM_WRITE_REQ`、`DBG_START_APP_REQ` 等，见 `protocol.rs`）。启动状态机用它读芯片修订号、写系统配置、上传固件、启动应用。
- **应用固件阶段**（固件跑起来之后）：`begin_lmac_mailbox`（`mailbox.rs:178`）发 LMAC 命令，目标是 `TASK_MM` 或具体功能任务，消息号是 `0x00xx`/`0x18xx` 的协议命令（`MM_SET_STACK_START_REQ` 等）。

两条通道共用同一个 `MailboxState`——帧格式相同，只是帧头和消息号不同。

## 3.6 消费方怎么接

`complete_mailbox`（`mailbox.rs:132`）看设备当前处于什么生命周期，决定把结果交给谁：

```rust
if self.lifecycle.state == AicState::Starting {
    self.complete_startup_mailbox(result)   // 启动状态机继续（第 4 章）
} else if let Some(control) = self.lifecycle.control.as_mut() {
    control.commands.pop_front();           // 控制命令队列弹出一条
    if control.commands.is_empty() {
        self.lifecycle.control = None;
        self.data.events.push_back(AicEvent::ControlComplete);  // 整个操作完成
    }
    Ok(())
} else { Err(AicError::CompletionMismatch) }
```

启动阶段，回应推着启动状态机往前走；就绪阶段，控制命令队列（第 7 章）弹出一条，队列空了就发 `ControlComplete`。`mailbox` 只在两条路径的"发命令"环节出现，命令队列和启动流程是它的两个消费者。

## 3.7 对比旧实现

旧 `send_cmd`（重构前 `fdrv/protocol/cmd.rs`）是**一个阻塞函数**：写 FIFO、然后 `yield_now` 循环读状态、直到回应或超时。新 mailbox 与它做的事完全一样，区别只在结构：

| | 旧 | 新 |
| --- | --- | --- |
| 等待信用 | 函数内 yield 循环 | Flow 阶段 + `retry_at` 数据 |
| 等待回应 | 函数内 yield 循环 | Count 阶段 + `retry_at` 数据 |
| 取消 | 无法插入 | `cancel_pending` 在下一轮 advance 变成 `AbortSdio`（第 7 章） |
| 超时 | 循环计数 | 绝对 deadline 比较 |
| 可测试 | 需要真实调度 | 喂假时间即可穷举（`progress.rs` 末尾测试直接驱动 mailbox） |

结论：mailbox 状态机把"发一条命令"从一个不可分割的阻塞调用，变成了 6 个可随时被打断、随时可被检查、随时可被取消的步骤。这是第 1 章原则在命令通道上的完整落地。
