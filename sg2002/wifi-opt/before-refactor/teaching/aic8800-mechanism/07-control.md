# 第 7 章 WiFi 控制操作：命令队列、取消与超时

> 对照代码：
> - `drivers/net/aic8800/src/device/control.rs`（操作 → 命令队列的翻译）
> - `drivers/net/aic8800/src/rdif/device/endpoints/control.rs`（适配层 WifiControl）
> - `drivers/net/aic8800/src/device/mailbox.rs`（每条命令的执行，第 3 章）
> - `net/ax-net/src/queue_runtime/executor/wifi.rs`（运行时的驱动方式）

## 7.1 一个"操作"不是一条命令

用户视角的 WiFi 操作（连接网络、开热点）在固件协议里是**一组有顺序的命令**。核心的 `ControlState` 就是一个命令队列（`control.rs`）：

```rust
pub(super) struct ControlState {
    pub commands: VecDeque<ControlCommand>,
    _wpa_nonce: Option<Entropy>,   // 安全连接时保留调用方的熵
}

pub(super) struct ControlCommand {
    pub message_id: u16,            // 请求消息号
    pub destination: u16,           // 固件目标任务号
    pub expected_message_id: u16,   // 期待回应的消息号（= 请求号 + 1）
    pub payload: Vec<u8>,           // 协议载荷
}
```

`build`（`control.rs`）把 `ControlRequest` 翻译成队列：

- `Connect` → 一条 `0x1800` 命令（载荷：SSID、信道、加密标志）。密码非空时要求 `entropy` 存在（`EntropyUnavailable` 错误，见下）。
- `Disconnect` → 一条 `0x1803` 命令。
- `StartOpenAccessPoint` → **五条命令**：`0x0006`（添加接口，携带 MAC）→ `0x0002`（配置）→ `0x000e`（设置波特率）→ `0x1c08`（下发 Beacon 帧内容）→ `0x1c00`（启动 AP，含频率/信道）。开热点在固件侧就是这么一整套流程，每条之间依赖前一条成功。

队列在 `drive_ready` 里被消费（`data_plane.rs:14`）：

```rust
if let Some(control) = self.lifecycle.control.as_ref()
    && let Some(command) = control.commands.front()
{
    let message_id = command.message_id;
    // ...
    self.begin_lmac_mailbox(message_id, destination, &payload, expected, now);
    return self.drive_mailbox(now);
}
```

即：队头命令发起 mailbox（第 3 章的六阶段机器），mailbox `Complete` 时 `complete_mailbox` 弹掉队头、发起下一条；队列空则发 `AicEvent::ControlComplete`（`mailbox.rs:132`）。

## 7.2 熵的要求

`Connect` 带密码时，`build` 检查 `entropy`（32 字节）：

```rust
if !password.is_empty() {
    nonce = Some(entropy.ok_or(AicError::EntropyUnavailable)?);
}
```

设计文档写明原因：

> 安全连接必须携带调用方拥有的 32 字节熵。熵缺失返回 `EntropyUnavailable`，禁止用时间戳代替随机源。

密码连接需要真随机数（WPA 握手），而**核心没有随机数发生器**——这是核心零依赖原则的必然结果。随机性必须由调用方（运行时，有硬件随机源）提供。这个检查把"没有随机源的平台"从运行期故障提前变成了构造期错误。适配层 `map_wifi_operation` 也做同样检查（在操作进入核心之前）。

## 7.3 适配层：两道小环 + 两层 deadline

操作从 ax-net 到达核心要跨两道环（`rdif/device/shared.rs` 的 `WifiChannels`）：

```rust
let requests = HeapRb::new(2);    // 操作请求：外层 → owner（容量 2）
let progress = HeapRb::new(8);    // 进度通知：owner → 外层（容量 8）
```

`AicWifiControl::start`（`endpoints/control.rs`）：把操作翻译成 `ControlRequest`，推进 requests 环，然后返回 `WifiControlProgress::WaitForInterruptUntil { deadline_nanos }`——**start 本身不执行任何硬件动作**，执行在 owner 的下一次 advance 里（`AicOwner::advance_with_cause` 的 `wifi_requests.try_pop()` 分支）。

`AicWifiControl::advance`：从 progress 环弹进度。进度有三种：`Complete`（操作完成）、`WaitForInterrupt`（继续等）、`RetryAt { deadline }`（核心在等一个具体时刻）。适配层把它们统一成"带 deadline 的等待"返回给 ax-net——**deadline 取两者较小值**（`inner_deadline.min(deadline_nanos)`），保证无论核心的 mailbox 超时（5 秒）还是控制超时（默认 30 秒）先到，外层都能按时醒来发现。

两层 deadline 的分工：

- **核心层**：`MAILBOX_TIMEOUT`（5 秒）——单条命令的响应超时，属于协议知识。
- **适配层**：`control_timeout`（默认 30 秒）——整个操作（可能含多条命令）的端到端超时，属于策略。

超时后 `advance` 返回 `NetError`，ax-net 把错误交给当初发起操作的调用方。

## 7.4 取消：一条路径打穿所有层

取消（`AicWifiControl::cancel`）只做一件事：往 requests 环推进一个 `ControlRequest::Cancel`。之后的一切由已有机制接力：

1. owner 弹出 Cancel，作为事件喂给核心 `consume_input`（`progress.rs:68`）；
2. 核心 `consume_control`（`progress.rs`）看到 Cancel：置 `cancel_pending = true`；如果此刻没有飞行中的 SDIO 请求，立即 `finish_cancel`；
3. 如果有一个请求在飞（mailbox 六个阶段的任意一步），下一轮 advance 的第④步检查（第 2 章的固定顺序）把它变成 `AbortSdio { request_id }`；
4. 适配层执行 abort：`ActiveOperation::abort`（`rdif/owner/operation.rs`）调用协议层的 `abort_*`，**中止的语义由 sdmmc 契约保证**——abort 返回时控制器已不再访问任何相关内存；
5. 适配层把 `SdioFailure::Aborted` 作为完成结果喂回核心（`consume_action` 的 `AbortSdio` 分支）；
6. 核心 `consume_completion` 看到 cancel_pending 且结果是 Aborted：`finish_cancel` 清掉 mailbox、control，发 `AicEvent::ControlCancelled`；
7. 适配层 `consume_event` 把 `ControlCancelled` 转成 progress 环上的 `Complete`，外层拿到"已取消"。

关键点：**取消不需要打断任何正在运行的代码**（没有可打断的东西），它只是让状态机在下一个检查点转向。中止针对"恰好正在飞的那一个请求"（`request_id` 精确匹配），这是单飞行规则的直接受益。

取消在启动阶段同样有效：`finish_cancel` 里 `if self.lifecycle.state == AicState::Starting` 分支清掉 startup 并回到 `Stopped`。

## 7.5 ax-net 怎么驱动这套东西

`executor/wifi.rs` 的 `run_wifi_step` 展示了运行时侧的统一模式：

```rust
fn run_wifi_step(group: &mut QueueGroupExecutor,
                 step: impl FnOnce() -> Result<WifiControlProgress, NetError>)
    -> Result<(WifiControlProgress, bool), NetError> {
    group.group.irq_control.quiesce()?;      // 先静默中断域（mask）
    let progress = step();                   // 执行一步（start 或 advance）
    let rearm = group.group.irq_control
        .rearm_and_check(ax_hal::time::monotonic_time_nanos());  // 原子重开+复查
    // ...
}
```

每一步控制推进都包裹在 **quiesce → 步进 → rearm** 里：执行控制操作期间 mask 掉中断（第 5 章讲的 CARD_INT 电平信号），步进结束后 `rearm_and_check` 原子地重开并检查"期间是否又有卡事件"——有就返回 `WorkPending`，group 立即再排一轮；没有就安静入睡。这保证了控制操作不会因为"执行期间的中断"而漏掉数据。

外层还有 `WifiWait` 状态机（`executor/wifi.rs` 的 `ActiveWifiRequest`）：`InterruptUntil` 表示"等中断，但 deadline 之前必须醒来"；`Deadline` 表示"等一个时刻"。ax-net 用 group 的 notify（带超时等待，第 8 章的 `wait_startup_deadline` 同类机制）实现这两种等待，而不是开新线程或自旋。

## 7.6 小结

- 操作 → 命令队列 → 逐条 mailbox：三个层次，每层只处理自己的一层抽象。
- 熵是调用方的义务，缺了就报错，不猜不凑。
- 进度与请求走两道小环，适配层负责 deadline 收敛（取最小值）。
- 取消是一条 `Cancel` 消息 + 既有检查点接力，全程无"打断"。
- ax-net 对每一步控制推进统一做 quiesce → 步进 → 原子 rearm，保证中断窗口闭合。
