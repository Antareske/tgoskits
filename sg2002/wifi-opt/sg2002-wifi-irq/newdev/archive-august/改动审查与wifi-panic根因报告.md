# SG2002 WiFi IRQ：改动审查与 panic 根因报告

- 分支：`sg2002/wifi-irq`（HEAD == dev，全部改动在工作区未提交）
- 材料：`www/logs/4.log`（当前修复后，立即 panic）、`www/logs/dev-wifi.log`（dev 分支，60s 超时 panic）
- 对照基线：dev 分支（`7c5bbd133`）、重构前的板测驱动（`0fc626fa4^`）、厂商 Linux 驱动（`www/LicheeRV-Nano-Build/osdrv/extdrv/wireless/aic8800`）

---

## 0. 摘要

1. dev 分支的 wifi 初始化 60s panic 的根因是：`advance_init_request` 提交下一个纯寄存器总线操作（PowerOn 起）后直接返回 Pending 而未推进，而协议层把这类操作标记为"等待中断"，但纯寄存器操作从不产生中断 → 死等直到 `DEFAULT_STARTUP_TIMEOUT`（60s）。工作区中的循环修复方向正确。
2. 修复放通了 SDIO 枚举与固件启动，流程首次走到 bootrom 邮箱（ReadRevision，`DBG_MEM_READ_REQ@0x4050_0000`）。4.log 的立即 panic（启动后约 180ms）发生在第一次邮箱应答读取：读回的帧未通过 `confirmation_payload` 校验（`frame[4..6] != 0x0401`），新邮箱流将其判为致命错误。
3. 新邮箱流缺少两个"板测实测"逼出来的健壮性行为，而旧板测驱动与厂商 Linux 驱动都具备：
   - **读到不匹配帧应重试而不是判死**：旧驱动 `poll_for_response` 对 `unexpected response id` 是 warn + continue；厂商驱动 IRQ 路径按消息 id 匹配分发、消费一切帧。
   - **邮箱交互前/后需排空残留数据**：旧驱动有 `drain_initial_stale_data` / `drain_post_init_data` / `drain_stale_data`。D80 在 wakeup、功能使能、中断使能之后，bootrom 可能先推一条非 CFM 帧，导致第一次 Count 轮询即读到非期望帧。
4. 诚实边界：4.log 拍摄于诊断日志（`MalformedMailboxFrame`、`aic: mailbox read … head=`）加入之前，坏帧的具体字节尚未实测。下次板测的 head 字节将直接证实本报告的推断（全零 = 残留空帧；具体 id = bootrom 消息）。

---

## 1. 当前改动 vs dev 全景

共 10 个文件，+147/−59。**真正改变行为的只有两处**，其余为诊断与注释。

| 文件 | 性质 | 内容 |
|---|---|---|
| `drivers/blk/sdmmc-protocol/src/sdio/io/init.rs` | **功能修复** | `advance_init_request` 改为循环：每提交一个新步骤立即用 `ProgressCause::Submitted` 推进一次；`submit_init_state` 增加一行 info 日志 |
| `net/ax-net/src/queue_runtime/mod.rs` | **功能改进** | `QueueInit(NetError)` 携带真实失败原因；builder 从 `startup_error` 取回错误 |
| `net/ax-net/src/queue_runtime/executor/mod.rs` | **功能改进** | `ExecutorControl` 增加 `startup_error: SpinLock<Option<NetError>>`，initialize 失败时记录；失败路径加 error 日志 |
| `drivers/net/aic8800/src/device/mailbox.rs` | 诊断 | 邮箱计数/读取日志；`confirmation_payload` 失败映射为带帧头字节的 `MalformedMailboxFrame` |
| `drivers/net/aic8800/src/device/model.rs` | 诊断+注释 | 新增 `MalformedMailboxFrame` 错误变体；中文学习注释 |
| `drivers/net/aic8800/src/device/owner.rs` | 注释 | 中文学习注释 |
| `drivers/net/aic8800/src/device/progress.rs` | 注释 | 中文学习注释 |
| `drivers/net/aic8800/src/device/startup/mod.rs` | 诊断 | `set_startup_stage` 增加 info 日志 |
| `drivers/net/aic8800/src/rdif/owner/progress.rs` | 诊断 | IRQ 快照日志、submit sdio 日志 |
| `Cargo.lock` | **无关** | axhvc/axivc 依赖移除，本地构建特性差异所致，与本次修复无关，提交时应排除 |

### 1.1 对功能改动的评述

**init.rs 循环（正确）**：与 sdhci 主机层语义严格吻合——纯寄存器总线操作（Reset/PowerOn/PowerOff/SetClock/SetBusWidth/SetSignalVoltage）在 `sdhci-host/src/host2/bus.rs` 的 `advance_host2_bus_state` 中：
- `AcknowledgedIrq` → 转 `RegisterPending`（中断不是这类操作的正确触发源）；
- 内部返回 `WaitingForIrq` → 一律改写为 `RegisterPending`（走轮询，永不等中断）。

因此"提交后立即推进一次"让这些操作要么一步完成（PowerOn/SetBusWidth），要么进入正确的轮询重试路径（Reset/SetClock 状态机）。对 COMMAND 步骤（CMD5/3/7/52）多出的一次 `advance(Submitted)` 是无害 no-op：主机层在 `!acknowledged && !command_needs_register_retry()` 时直接返回 `WaitingForIrq`（`sdhci-host/src/host2/transaction.rs:217-219`），不消耗任何 IRQ 状态。第一轮使用调用方 cause、后续轮使用 `Submitted` 的处理也正确。

**ax-net 错误透传（合理）**：`QueueInit(NetError)` 让 `devices.rs:97` 的 panic 携带真实原因，正是 4.log 里能看到 "AIC core failed: AIC mailbox response was malformed" 的原因。两个次要注意点：
- `unwrap_or(NetError::NotSupported)` 兜底：若 executor 因 affinity 失败（此时 `affinity_status` 置 FAILED 而 `startup_status` 保持 PENDING），`wait_status` 会永久自旋（dev 已有行为，非本次引入），且记录不到真实错误。
- `startup_error` 的 `lock_irqsave` 仅在 executor 任务上下文使用，与 builder 侧读取构成简单同步，无锁序问题。

**其他**：`sdio: init state {:?}` 建议降 debug 级；`MalformedMailboxFrame` 是对公共错误枚举 `AicError` 的扩展，属有意的可诊断性变更。

---

## 2. 驱动启动链路（关键路径）

```
axruntime::init_net (os/arceos/modules/axruntime/src/devices.rs:97，panic 点)
└─ NetworkRuntimeBuilder::build (net/ax-net/src/queue_runtime/mod.rs)
   ├─ 注册 wlan0 IRQ（dev-wifi.log 7.924706 "registered wlan0-g0-s0 IRQ"）
   ├─ 派发 executor 任务 → queue_executor_main
   │  └─ group.initialize() → AicOwnerStartup (rdif/device/endpoints/startup.rs)
   │     └─ AicOwner::start (rdif/owner/progress.rs:84)
   │        ├─ enable_completion_irq
   │        ├─ SdioCard::submit_init + advance 循环（io/init.rs，本次修复点）
   │        └─ AicDevice::start → drive_startup（device/startup/mod.rs）
   │           ├─ EnableFunction → SetBlockSize → EnableFunctionInterrupt
   │           ├─ VendorSetup(0..3)：fn0 0xF2=0x7F；byte_mode 0x07=1；wakeup 0x02=0x11
   │           ├─ VendorDelay(5ms) → VendorReady（读 sleep 0x01 ready 位 0x10）
   │           ├─ ReadRevision：第一条邮箱命令 DBG_MEM_READ_REQ@0x4050_0000 ← 4.log panic 发生于此
   │           ├─ …（SystemConfig/UploadMain/UploadPatch/…）
   │           ├─ StartApplication：DBG_START_APP_REQ@MAIN_ADDRESS
   │           └─ … → StackStart（MM_SET_STACK_START_REQ，固件启动后 LMAC 邮箱）
   └─ wait_status(startup_status)；非 READY → panic
```

邮箱单次交互（`device/mailbox.rs` 的 `drive_mailbox`）：`Flow`（读 flow_control 0x03）→ `Write`（写 512B 命令帧到 write_fifo 0x10）→ `Settle`(2ms) → `Count`（读 block_cnt 0x04，`interrupt_block_count`：位 7 置位→重试，否则位 0-6 = 块数）→ `Read`（读 count×512B 自 read_fifo 0x0F）→ `confirmation_payload` 校验。

板上芯片为 **Aic8800D80（V3）**：`drivers/ax-driver/src/net/aic8800/fdt.rs` 的 `chip_variant()` 缺省即 `aic8800d80` 且仅支持它；寄存器表与厂商 V3 定义逐一吻合（见附录 B）。D80 的命令邮箱走 function 1（旧驱动 `cmd_func` 真机注释：仅 DC/DW 走 func2），与 `command_function() = 1` 一致。

---

## 3. dev 分支 60s panic 机制

### 3.1 证据链

1. dev-wifi.log：SD 卡初始化正常完成（7.713 "init done kind=Sd"），FS 挂载正常，wlan0 IRQ 注册（7.924706）后无任何 wifi 日志，最终 panic `network queue initialization failed`（旧 `QueueInit` 无 payload，故无详情）。
2. 60s 来源：`DEFAULT_STARTUP_TIMEOUT = Duration::from_secs(60)`（`drivers/net/aic8800/src/rdif/device/endpoints/device.rs:30`）。`AicOwnerStartup::finish` 把 `OwnerProgress::Wait(Interrupt)` 映射为 `WaitForInterruptUntil { deadline: 启动时刻+60s }`（`endpoints/startup.rs:46-70`）；executor 的 `initialize()` 对此走 `wait_startup_deadline`（`executor/mod.rs:208-211`）；到期后 `startup.advance` 检测超时 → `SdioFailure::Timeout`（`startup.rs:84-96`）→ `STATUS_FAILED` → builder panic。
3. 死等点：
   - `ProtocolHost::submit_bus_op` 提交时无条件 `self.progress_wait = HostProgressWait::Irq`（`drivers/blk/sdmmc-protocol/src/sdio/transport.rs:278`）。
   - 旧 `advance_init_request`：完成一步后 `submit_init_state` 提交下一步，**随后直接 `return Ok(Pending)`，不推进新步骤**。
   - owner 随后调用 `protocol_wait()`，读到的正是 submit 时写入的 `Irq` → `OwnerProgress::Wait(Interrupt)` → executor 死等一个纯寄存器操作永不产生的中断。
4. 卡死步骤：ResetAll 之后提交的 **PowerOn**（首个"提交后不推进"的纯寄存器操作；ResetAll 因 `start()` 用 `Submitted` 推了第一把而幸免）。

### 3.2 结论

dev 的 60s panic = "提交纯寄存器总线操作后等中断"的死锁，由 60s 启动超时兜底引爆。工作区的循环修复（每步提交后立即以 `Submitted` 推进一次）正是针对该死锁的正确解法，且与主机层 register-only 语义严格对齐（见 1.1）。

---

## 4. 4.log 立即 panic 根因

> **【已被修正（superseded）】**：本节"邮箱坏帧"根因推断被板测 5.log/6.log 与宿主机单元测试推翻——4.log/5.log/6.log 的 `MalformedResponse` 实际出自 VendorSetup 阶段的响应形状失配（`expect_unit` vs 适配层 Direct→`Byte`），从未到达 ReadRevision 邮箱。详见 `www/newdev/VendorSetup响应形状失配.md`。本节保留原貌仅作推理过程记录；坏帧重试/排空方案的定位相应修正为"邮箱阶段的预防性加固"。

### 4.1 时间线（4.log）

| 时刻 | 事件 |
|---|---|
| 7.441249 | 串口控制台激活 |
| 7.441727–7.455123 | SD 卡（cvsd）ResetAll → PowerOn → SwitchVoltage → 1-bit → Identification 时钟 |
| 7.478446 | "detected Sd ocr=0xc1ff8000"（SD 记忆卡识别，此后 4.log 无 CSD/FS 日志，见 6.3） |
| **7.62406** | **panic：`failed to initialize network queue runtime: network queue initialization failed: Other error: AIC core failed: AIC mailbox response was malformed`** |

控制台激活到 panic 仅约 183ms——"立即 panic"。panic 路径：`AicError::MalformedResponse` ← 邮箱读取帧未通过 `confirmation_payload` 校验 ← `fail()` ← executor 启动失败 ← `devices.rs:97` panic。

### 4.2 到达 panic 意味着什么

`MalformedResponse` 只可能出自 `MailboxRead` 分支（`device/mailbox.rs:123-145`，`confirmation_payload` 失败）或 `expect_*` 类型不匹配（此处不可能，Direct 操作恒返回 Byte、DMA 读恒返回 Data）。因此可以断定：**SDIO 枚举、芯片身份校验、startup 各阶段全部成功走通，CMD52/CMD53/IRQ 通路无恙**——修复确实放通了整条 init 链路；故障精确定位在第一条邮箱命令（ReadRevision，期望 CFM `0x0401`）的应答帧内容不匹配：`frame[4..6] != 0x0401`（帧长 ≥512，不可能是"帧过短"分支）。

### 4.3 根因：新邮箱流缺少两个板测实测的健壮性行为

**行为一：读到不匹配帧应消费后重试，而不是判死。**

- 新代码（`device/mailbox.rs` MailboxRead 分支）：校验失败 → `AicError` → `fail()` → 立即 panic。
- 旧板测驱动（`0fc626fa4^:drivers/net/aic8800/src/fdrv/core/init.rs` 的 `poll_for_response`）：`read_and_parse_response` 返回 `unexpected response id` 时 `log::warn! + continue`，继续轮询 `RESPONSE_MAX_RETRY` 次——坏帧已被读走，下一轮 count 轮询即面对真正 CFM。
- 厂商 Linux 驱动（`aicsdio.c` D80 IRQ 路径）：`while (intstatus)` 把 FIFO 内所有帧读尽，按消息 id 匹配分发；不匹配的帧自然丢弃。

**行为二：邮箱交互前/后需要排空残留数据。**

- 旧板测驱动在固件启动前后各有 `drain_initial_stale_data`（init.rs:19，调用于 :566）、`drain_post_init_data`（:101，调用于 :572）、`drain_stale_data`（:312）：轮询 block_cnt，非零即读走丢弃（最多 10 轮），位 7（OTHER）置位时跳过。
- 新代码完全没有 drain。D80 在 wakeup（0x02=0x11）、功能使能、中断使能之后，bootrom 可能先推一条非 CFM 帧（boot 状态消息等）。于是第一条邮箱的第一次 Count 轮询就看到 `block_cnt > 0`，读回的正是那条非期望帧 → 校验失败 → 立即 panic。这正好解释"立即"而非"超时"的形态。

### 4.4 排除的替代解释

- **帧布局偏移错误**：新 `confirmation_payload`（`protocol.rs:65-77`：id 在 [4..6]、长度在 [10..12]、payload 在 16）与旧板测驱动 `read_and_parse_response`（`PROTO_HEADER_SIZE = 16：SDIO(4) + LMAC(12, 含 pattern)`）逐字节一致，属板测验证过的布局。
- **寄存器地址错误**：D80 寄存器表（flow 0x03 / block_cnt 0x04 / rd 0x0F / wr 0x10 / sleep 0x01 / wakeup 0x02 / int_en 0x00）与厂商 V3 定义及旧驱动逐一吻合；且 Count 读到非零说明该寄存器读数有效。
- **读 0x4050_0000 本身非法**：厂商 `aicbsp_driver_fw_init` 与旧驱动 `fw/chip/config.rs:18` 均通过 bootrom 调试邮箱读该地址（chip_rev），属正常用法。
- **SDIO 总线/时钟问题**：读操作本身成功返回 512 字节，说明电气层无恙。
- **帧尾截断**：读长 = count×512 ≥ 512，不会是 len<16 分支。

### 4.5 诚实边界

4.log 的 panic 文本为旧错误变体 `AIC mailbox response was malformed`，且日志中没有任何 `aic:` 前缀诊断行——说明 4.log 拍摄于 `MalformedMailboxFrame` 与 mailbox 诊断日志加入之前。坏帧的确切字节（是全零残留帧，还是某条 bootrom 消息 id）尚未实测。下一次板测输出 `aic: mailbox count raw=` 与 `aic: mailbox read … head=` 后即可证实：head 全零 → 残留空帧；出现具体 id（如 0x0xxx）→ bootrom 消息。

---

## 5. 修复建议

1. **坏帧重试（核心修复）**：`consume_mailbox_response` 的 `MailboxRead` 分支中，`confirmation_payload` 失败时不要 `fail`，改为：
   - 用新增的 `MalformedMailboxFrame` 数据打一条 warn（保留可观测性）；
   - `mailbox.phase = MailboxPhase::Count`，`mailbox.retry_at = Some(now + MAILBOX_FLOW_RETRY)`（1ms），继续轮询真正的 CFM；
   - 由现有 5s `mailbox.deadline`（`MAILBOX_TIMEOUT`）兜底超时。
   语义与旧驱动 `poll_for_response` 的 continue、厂商驱动的按 id 匹配一致。
2. **残留排空（加固）**：在 `MailboxFlow` 之前（或 Count 首次轮询时）按旧驱动做法排空：轮询 block_cnt，`>0` 读走丢弃，循环有限次；位 7 置位直接跳过。
3. **测试先行（项目规范要求）**：`AicDevice::advance` 是纯函数式核心，可直接单测"读到不匹配帧 → 返回重试动作而非 `Failed`"。先在错误实现上验证测试失败，再修复，再验证通过。
4. **提交注意**：`Cargo.lock` 的 axhvc/axivc 移除与本次修复无关，不要随修复提交；`sdio: init state` 日志降 debug；诊断日志在板测确认后按需保留或降级。

---

## 6. 连带观察项

1. **OTHER 位（位 7）处理**：`interrupt_block_count`（`registers.rs:82-87`）遇位 7 置位只重试、不按厂商方式清 `sleep_reg(0x01)` bit0（dev-to-host soft irq）。若 bootrom 保持该位直到主机清除，Count 将永远重试 → 5s `MailboxTimeout`。下次板测观察 `aic: mailbox count raw=` 是否长期出现 `0x8x`；必要时在 Count 分支增加 sleep_reg 读改写清除（V3 特有）。
2. **flow_credits 掩码**：新代码对 D80 也按 0x7F 掩码（`registers.rs:78-80`），厂商对 D80 不掩码（`aicsdio.c:659-690`）。若 D80 flow_ctrl_q1 位 7 有含义，掩码后 `0x80` 会当 0 → 重试（安全方向），目前无害，板测留意 flow 值即可。
3. **每笔 TX 前的 wakeup**：厂商在 `CONFIG_SDIO_PWRCTRL` 下每笔 TX 前 `aicwf_sdio_wakeup()`（写 wakeup 0x11、轮询 sleep ready），并在 boot 后写 `wakeup_reg = 4`。新驱动只有 VendorSetup 阶段的一次 wakeup 写入。对 bootrom 阶段的首条邮箱无影响，但后续 mailbox（StartApplication、StackStart）若芯片进入睡眠会表现为 flow 恒 0 → 超时。列为后续板测观察项。
4. **cvsd SD 卡初始化停在 "detected Sd" 的现象（4.log）**：本次修复未改 SD 记忆卡状态机（`sdio/init/state_machine.rs`），dev 上它正常完成。4.log 中 CMD9(CSD) 读（dev 上约 2ms）在 146ms 内未完成、FS/`Primary CPU 0 init OK` 日志整体缺失，而 panic 又来自其后的 `init_net`——最可能是串口捕获丢行，或 smp=1 单核上 boot 任务 `wait_status` 自旋与 wifi executor 密集 IRQ 抢占 block 任务（0:9/0:10）。修掉 mailbox panic 后自然可复验；若复现再查调度。
5. **executor affinity 失败路径**：affinity 失败时 `startup_status` 保持 PENDING，builder `wait_status` 永久自旋（dev 已有行为，非本次引入）；`startup_error` 的 `unwrap_or(NetError::NotSupported)` 在"失败但未记录"场景会给出误导错误。

---

## 附录 A：日志摘录

**4.log（关键行）**

```
[  7.441727 0:9 sdmmc_protocol::sdio::init::state_machine:102] sdio: submit bus op ResetAll
[  7.443150 0:10 sdmmc_protocol::sdio::init::state_machine:102] sdio: submit bus op PowerOn
[  7.453457 0:10 sdmmc_protocol::sdio::init::state_machine:102] sdio: submit bus op SwitchVoltage(V330)
[  7.454982 0:10 sdmmc_protocol::sdio::init::state_machine:63] sdio: submit bus op SetBusWidth(Bit1)
[  7.455123 0:10 sdmmc_protocol::sdio::init::state_machine:63] sdio: submit bus op SetClock(Identification)
[  7.478446 0:10 sdmmc_protocol::sdio::init::state_machine::identify:175] sdio: detected Sd ocr=0xc1ff8000
[  7.62406] panicked at os/arceos/modules/axruntime/src/devices.rs:97:29:
failed to initialize network queue runtime: network queue initialization failed:
  Other error: AIC core failed: AIC mailbox response was malformed
```

**dev-wifi.log（关键行）**

```
[  7.667834 0:9 …] sdio: submit bus op ResetAll
…（SD 卡初始化全部完成）
[  7.713639 0:10 …] sdio: init done kind=Sd … host_bus_width=Bit4 …
[  7.713681 0:10 sdmmc_protocol::rdif::queue:173] sdmmc block init complete: …
[  7.921933 0:2 ax_fs_ng:108]   filesystem type: "ext4"
[  7.922424 0:2 ax_runtime:343] Primary CPU 0 init OK.
[  7.924706 0:2 ax_runtime::irq:193] registered wlan0-g0-s0 IRQ … HwIrq(38) …
[ panicked at os/arceos/modules/axruntime/src/devices.rs:97:29:
failed to initialize network queue runtime: network queue initialization failed
```

（dev 的 panic 无详情，因为当时 `QueueInit` 不携带 `NetError`；60s 超时由 `DEFAULT_STARTUP_TIMEOUT` 机制给出，与日志断流吻合。）

## 附录 B：代码引用索引

**本次修复相关**

- `drivers/blk/sdmmc-protocol/src/sdio/io/init.rs:119-154`：新的 `advance_init_request` 循环（修复本体）
- `drivers/blk/sdmmc-protocol/src/sdio/transport.rs:275-283`：`submit_bus_op` 提交时 `progress_wait = Irq`（dev 死等根源）；`:285-318` `advance_bus_op`
- `drivers/blk/sdhci-host/src/host2/bus.rs:120-165`：register-only 语义（`AcknowledgedIrq`→`RegisterPending`；`WaitingForIrq`→`RegisterPending`）
- `drivers/blk/sdhci-host/src/host2/transaction.rs:206-295`：`advance_transaction` 的 cause 语义
- `drivers/net/aic8800/src/rdif/device/endpoints/device.rs:30`：`DEFAULT_STARTUP_TIMEOUT = 60s`
- `drivers/net/aic8800/src/rdif/device/endpoints/startup.rs:46-70, 84-96`：`WaitForInterruptUntil` 映射与超时判死
- `net/ax-net/src/queue_runtime/executor/mod.rs:198-249`：`initialize()` 启动循环；`:424-430` `startup_error`
- `net/ax-net/src/queue_runtime/mod.rs:163-166, 471-476, 590-611, 755-759`：`QueueInit(NetError)`、记录与取回、`wait_status`

**邮箱流（新代码）**

- `drivers/net/aic8800/src/device/mailbox.rs:15-23`：邮箱相位；`:36-78` `drive_mailbox`；`:80-149` `consume_mailbox_response`（MailboxRead 分支 :123-145 为判死点）
- `drivers/net/aic8800/src/device/startup/mod.rs:21-44`：启动阶段表；`:122-129` ReadRevision（第一条邮箱）
- `drivers/net/aic8800/src/device/startup/vendor.rs:8-24`：V3 vendor 写入序列（fn0 0xF2=0x7F、byte_mode=1、wakeup=0x11）
- `drivers/net/aic8800/src/device/progress.rs:77-86`：核心对 Irq 事件仅记账；`:238-240` `command_function() = 1`
- `drivers/net/aic8800/src/protocol.rs:65-77`：`confirmation_payload`（id [4..6]、长度 [10..12]、payload 自 16）
- `drivers/net/aic8800/src/registers.rs:46-61`：V3 寄存器表；`:82-87` `interrupt_block_count`
- `drivers/ax-driver/src/net/aic8800/fdt.rs:90-96`：芯片变体默认 `aic8800d80`，仅支持 d80

**旧板测驱动（`git show 0fc626fa4^:drivers/net/aic8800/src/fdrv/core/init.rs`）**

- :72-79 `cmd_func`：真机注释 "DC/DW 的命令邮箱在 SDIO function 2(与 bootrom 阶段一致, 真机实测 CFM 的 block_cnt 在 func2 递增, func1 恒 0)。其余芯片走 func1"
- :135-163 `check_flow_control_polling`：`func == 2` 跳过流控
- :172-227 `poll_for_response`：位 7 重试、count=0 重试、**解析失败 warn + continue**
- :229-268 `read_and_parse_response`：`resp_id = [4..6]`；`param_offset = 16（SDIO(4) + LMAC(12, 含 pattern)）`
- :19/:101/:312 三个 drain 函数；:566/:572 调用点
- :526 "block_cnt(0x12) 不递增 → 命令轮询永远超时"；:543 "芯片永不驱动 RX, block_cnt(0x12) 恒 0 → 轮询永远超时"（板测经验注释）
- `fw/chip/config.rs:18`：旧驱动同样经 bootrom 调试邮箱读 `CHIP_REV_ADDR`(0x4050_0000)

**厂商 Linux 驱动（`www/LicheeRV-Nano-Build/osdrv/extdrv/wireless/aic8800`）**

- `aic8800_bsp/aicsdio.c:659-690`：`aicwf_sdio_flow_ctrl`（仅 8801/DC/DW 掩码 0x7F）
- `aic8800_bsp/aicsdio.c:749-795`：`aicwf_sdio_wakeup`（D80 写 0x11、轮询 sleep 0x10）
- `aic8800_bsp/aicsdio.c:1450-1490`：D80 IRQ 路径（读 misc_int_status 0x04；OTHER 位置位时清 sleep_reg bit0 **且仍读帧**；`intstatus>0` 读尽块）
- `aic8800_bsp/aicsdio.c:1799-1833`：`aicwf_sdiov3_func_init`（enable func 后 fn0 0xF2=0x7F）
- `aic8800_bsp/aic_bsp_driver.c:1674-1691`：`aicwifi_start_from_bootrom`（DBG_START_APP_REQ, HOST_START_APP_AUTO）
- `aic8800_bsp/aic_bsp_driver.c:1910-1930`：D80 启动序列（fw upload → patch → sys config → start app）
- `aic8800_bsp/aic_bsp_driver.c:2004-2027`：`aicbsp_driver_fw_init`（读 0x40500000，chip_rev = memdata>>16）
- `aic8800_fdrv/rwnx_msg_tx.c:4215-4233`：`rwnx_send_dbg_mem_read_req`（cmd_mgr 队列 + 按 id 匹配 CFM）

## 附录 C：三方对照表（邮箱应答处理）

| 行为 | 新代码（当前） | 旧板测驱动（0fc626fa4^） | 厂商 Linux |
|---|---|---|---|
| 读到不匹配帧 | 判死 → panic | warn + continue 重试 | 按 id 匹配，自然丢弃 |
| 发送前排空残留 | 无 | `drain_initial/post/stale` | IRQ 路径读尽所有帧 |
| 位 7(OTHER) 处理 | 仅重试 | 仅重试 | 清 sleep_reg bit0 后仍读帧 |
| 超时兜底 | 5s 邮箱超时 | `RESPONSE_MAX_RETRY` 轮询 | cmd_mgr 超时机制 |
| CFM 偏移 | id[4..6]/len[10..12]/payload@16 | 相同（板测验证） | 相同 |

---

## 实现记录（2026-08-28）

按报告 §5 方案实现，改动集中在 `drivers/net/aic8800/src/device/`：

1. **坏帧重试**（`mailbox.rs` MailboxRead 分支）：`confirmation_payload` 失败不再 `fail()`，改为 warn（携带 `MalformedMailboxFrame` 帧头字节）→ 相位回 `Count`、`retry_at = now + 1ms`，由 5s `MAILBOX_TIMEOUT` 兜底。
2. **残留排空**（`mailbox.rs` + `model.rs`）：新增 `MailboxPhase::DrainCount/DrainRead` 相位与 `IoPurpose::MailboxDrain`；每次 mailbox 开始先轮询 block_cnt（`interrupt_block_count`），`>0` 读走丢弃（上限 `MAX_MAILBOX_DRAIN_ROUNDS = 10`，位 7 置位/计数 0 → 结束排空）；`begin_debug_mailbox`/`begin_lmac_mailbox` 初始相位改为排空。
3. **排空读失败容忍**（`progress.rs` + `mailbox.rs::consume_drain_discard_failure`）：残留内容读回失败（旧驱动注释中"CRC error expected"的情形）跳过并继续排空，不判死；计数轮询失败仍按错误传播。
4. **研究性日志**：mailbox begin（id/expected/帧长）、drain 丢弃/结束、flow 信用值（info 成功 / debug 无信用，避免 1ms 重试刷屏）、坏帧 warn 含帧头字节。已有的 count raw、read head、startup stage、irq snapshot、submit sdio 日志保留。

**测试（先行）**：`mailbox.rs` 新增两条回归测试——
- `mailbox_drains_pending_fifo_frames_before_flow_control`：首个动作是排空计数轮询 → 残留块触发丢弃读 → 空 FIFO 后进入流控读；
- `mailbox_retries_after_an_unvalidated_frame_instead_of_failing`：坏帧 → `RetryAt`（非 `Failed`，`state() != Failed`）→ 重试窗口后重新轮询计数。

两条测试在旧实现上确认失败（12 通过 2 失败），修复后全部通过。验证：`cargo test -p aic8800 --features "host-test rdif"`（23 单测 + 集成全过）、`cargo xtask clippy --package aic8800`（3 项检查 -D warnings 全过）、`cargo fmt`。

**板测预期**：坏帧出现时打一条 `aic: mailbox frame did not validate; retrying: ...` warn 后继续等真正 CFM；残留帧被 `aic: mailbox drain: discarding ...` 提前清掉。若 5s 超时且 `count raw` 恒为 `0x8x`，进入 OTHER 位清理（清 `sleep_reg(0x01)` bit0）那一层。
