# 重构后的 AIC8800 机制 —— 教学文档

本系列讲解 dev 分支上重构后（PR #2201 `refactor(sdmmc): unify SDIO protocol and AIC8800 driver`）的 AIC8800 驱动机制。写作方式：解释"机制为什么这样运转、实际怎么运转"，而不是罗列"有哪些东西"。所有讲解都指向真实代码，可以边对照边读。

## 学习路线

| 章节 | 讲什么 | 主要对照代码 |
| --- | --- | --- |
| [01 驱动为什么不能再自己等](01-why-progress-state-machine.md) | 整个重构的核心思想：把"等待"从驱动里赶出去。驱动不再调用 sleep/yield/自旋，只报告"下一步需要什么" | `drivers/net/aic8800/src/device/model.rs`、`progress.rs` |
| [02 AicDevice 的运转规则](02-advance-input-output.md) | 状态机一次 advance 内部发生什么：输入怎么进来、action 怎么产出、为什么同一时刻只有一个 SDIO 请求在飞 | `device/progress.rs`、`device/owner.rs` |
| [03 固件命令怎么发出去](03-mailbox.md) | mailbox 状态机：写 FIFO、等流控信用、轮询回应、匹配消息号，以及为什么分这么多阶段 | `device/mailbox.rs`、`protocol.rs` |
| [04 启动状态机](04-startup.md) | 从裸卡到可收发，20 多个阶段各自对硬件做了什么 | `device/startup/mod.rs`、`startup/firmware.rs` |
| [05 中断与收包](05-irq-rx.md) | CARD_INT 电平信号、IRQ latch、一次中断如何同时推动两件事、FIFO 聚合帧解析 | `rdif/device/shared.rs`、`device/data_plane.rs`、`rx.rs` |
| [06 发包与 buffer 流转](06-tx-flow.md) | 协议层的 buffer 怎么过四道环、流控信用不足时怎么办、为什么核心只认识 TxToken | `rdif/device/queues.rs`、`rdif/owner/output.rs`、`tx.rs` |
| [07 WiFi 控制操作](07-control.md) | connect/AP 等操作怎么变成命令队列、取消如何在任意时刻生效、超时归谁管 | `device/control.rs`、`rdif/device/endpoints/control.rs` |
| [08 owner 转移与运行时配合](08-owner-runtime.md) | 唯一的 owner 如何从启动端点移交到 poll 端点、ax-net 怎么驱动它、sdmmc 事务契约 | `rdif/device/endpoints/startup.rs`、`rdif/owner/progress.rs`、`net/ax-net/src/queue_runtime/executor/wifi.rs` |

## 总览：新架构的一句话

驱动核心不再是一个"会自己行动的程序"，而是一个**纯状态机**：外界把"时间、硬件完成结果、中断、请求、数据"喂进来，它推进一步，然后说出"下一步需要外界做什么"。所有需要等待的地方，它都只是报出一个事实（"请在某时刻再叫我"或"请等中断"），**等待本身由外面的网络运行时执行**。这是理解全部章节的钥匙。

```
外界（ax-net 运行时 / 适配层）
   │  输入: 时间 + SDIO 完成 + IRQ 快照 + 控制请求 + TX 数据
   ▼
AicDevice（纯状态机，无锁、无线程、无时钟、无休眠）
   │  输出: AicAction —— SubmitSdio / AbortSdio / RetryAt(时刻) / WaitForInterrupt / Event / Idle
   ▼
外界执行 action，把结果作为下一次输入喂回来
```

四层分工见设计文档 `docs/design/unified-sdio-aic8800.md`：Driver Core（状态机）→ Capability Adapter（把 SDIO 卡、DMA、队列接到核心上）→ OS Glue（FDT/MMIO/时钟复位）→ Runtime（固定 CPU、IRQ 注册、调度、定时）。
