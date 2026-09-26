# 第 4 章 启动状态机：从裸卡到可收发

> 对照代码：`drivers/net/aic8800/src/device/startup/mod.rs`（全部阶段）、`startup/firmware.rs`（固件上传）、`startup/vendor.rs`（厂商寄存器）、`firmware.rs`（镜像常量）、`protocol.rs`（debug 命令）
> 前置章节：第 2 章（advance 规则）、第 3 章（mailbox）

## 4.1 启动要解决的物理问题

AIC8800 插上电之后芯片里只有 bootrom 能执行。要让它变成一块能收发 WiFi 帧的网卡，主 CPU 必须完成：**把应用固件写进芯片内存 → 让固件跑起来 → 用固件协议把网络协议栈（LMAC）架起来 → 打开中断**。每个步骤都是 SDIO 总线上的真实操作，有严格的先后顺序。启动状态机就是这条流水线的展开。

`startup/mod.rs:22` 的 21 个阶段：

```rust
pub(super) enum StartupStage {
    EnableFunction,             // SDIO 功能启用
    SetBlockSize,               // 设置 512 字节块
    EnableFunctionInterrupt,    // 启用功能中断
    VendorSetup(u8),            // 厂商寄存器序列（逐条写）
    VendorDelay,                // 写完等芯片就绪
    VendorReady,                // 读就绪状态
    ReadRevision,               // 读芯片修订号
    SystemConfig(usize),        // 写系统配置表（逐条）
    UploadMain(usize),          // 上传主固件（1KB 一块）
    UploadPatch(usize),         // 上传补丁固件
    ReadConfigBase,             // 读配置基址
    PatchMetadata(usize),       // 写补丁元数据（逐条）
    MaskedConfig(usize),        // 写掩码配置（逐条）
    SlowClock,                  // 降时钟到 400kHz
    StartApplication,           // 发启动命令
    FastClock,                  // 升时钟到 25MHz
    Stabilize,                  // 等 200ms
    Reinitialize(u8),           // 重新初始化功能寄存器
    StackStart,                 // 设置协议栈栈基址
    ArmChipInterrupt,           // 使能芯片中断
    Complete,
}
```

## 4.2 前半段：让 SDIO 通道可用

`EnableFunction`、`SetBlockSize`、`EnableFunctionInterrupt` 三个阶段发出的请求（`startup/mod.rs:63`）分别是 `SdioRequestKind::EnableFunction(1)`、`SetBlockSize { function: 1, block_size: 512 }`、`EnableFunctionInterrupt(1)`。

这三步属于 **SDIO 卡协议的标准动作**，不是 AIC 专有：SDIO 卡把功能划分成编号的 Function（AIC 的通信全走 Function 1）；每个 Function 要先启用、设好块大小，其中断才能上报。这三步的实现在 `sdmmc-protocol` crate 里（`SdioCard::submit_enable_function` 等），核心只是把它们排进流水线。这里体现第 1 章的分层：**卡协议知识在 sdmmc-protocol，AIC 芯片知识在 aic8800 核心，核心通过 `SdioRequestKind` 这种抽象的"愿望清单"使用前者**。

`VendorSetup`（`startup/vendor.rs`）写的是厂商规定的寄存器序列。D80 型号三条：

```rust
0 => Some(write_byte(0, 0xf2, 0x7f)),                  // 功能 0 的厂商控制字
1 => Some(write_byte(1, self.registers.byte_mode_enable, 1)),  // 开启 byte 模式
2 => Some(write_byte(1, wakeup寄存器, SDIOWIFI_V3_WAKEUP_VALUE)), // 唤醒值
```

每条一个 `write_byte`（CMD52），完成后进下一条；写完进 `VendorDelay`（V3 型号等 5ms），然后 `VendorReady` 读 sleep-status 寄存器，`interface_ready` 位不置位就报超时（`startup/mod.rs:229` 起）。**这个"写完→延时→回读确认"的三段式，是启动流程里所有芯片级步骤的通用模式。**

## 4.3 中段：写内存、传固件

芯片内存（firmware 的 `MAIN_ADDRESS = 0x0012_0000`）要通过 SDIO 写进去。bootrom 提供一组 debug 命令：`DBG_MEM_WRITE_REQ`（写一个 32 位字）、`DBG_MEM_BLOCK_WRITE_REQ`（写一块最多 1024 字节）、`DBG_MEM_MASK_WRITE_REQ`（掩码写）、`DBG_MEM_READ_REQ`（读）。

- `ReadRevision`：用 `DBG_MEM_READ_REQ` 读芯片修订地址，取回的值存进 `StartupState.revision`（校验必须是 1/3/7 之一，否则 `UnsupportedRevision`）。
- `SystemConfig`：`SYSTEM_CONFIG` 表里有 10 条 (地址, 值)，逐条 `DBG_MEM_WRITE_REQ`（`startup/firmware.rs` 的 `drive_system_config`）。
- `UploadMain`：主固件镜像（D80 约几百 KB）按 `UPLOAD_CHUNK = 1024` 字节一块，逐块 `DBG_MEM_BLOCK_WRITE_REQ` 写进 `MAIN_ADDRESS + offset`。**一块 = 一次 mailbox = 5 个 SDIO 操作**；几百 KB 就是几百轮。每轮之间没有任何"等待固件"的步骤——bootrom 逐块应答，状态机靠每次 mailbox 完成驱动下一个 offset。
- `UploadPatch`：同理（D80 补丁为空数组，`images()` 返回 `&[]`，该阶段直接跳过，见 `startup/firmware.rs` 的 `drive_patch_upload` 开头判断）。

`ReadConfigBase` 用 `DBG_MEM_READ_REQ` 读 `MAIN_ADDRESS + CONFIG_BASE_OFFSET`（即 0x120000 + 0x180）处的 4 字节配置基址，存进 `StartupState.config_base`。`PatchMetadata` 写补丁表：表项的地址值要加上这个 `config_base` 重定位（`startup/firmware.rs` 的 `patch_metadata` 里 `entry[0].wrapping_add(config_base)`）；`MaskedConfig` 写两张掩码写常量表。这三段是 8801 的固件装配路径，D80 不经过。

## 4.4 末段：启动应用、稳定期、重初始化

两个型号的末段路径不一样（`drive_startup` 与 `complete_startup_mailbox` 的分支）：

```text
8801:  ... → MaskedConfig → SlowClock(400kHz) → StartApplication → FastClock(25MHz)
      → Stabilize(200ms) → Reinitialize → StackStart → ArmChipInterrupt → Complete

D80:   ... → UploadMain → StartApplication → Stabilize(200ms)
      → Reinitialize → StackStart → ArmChipInterrupt → Complete
```

`SlowClock`（`SetClockHz(400_000)`）与 `FastClock`（`SetClockHz(25_000_000)`）只存在于 8801 路径：启动应用命令前后各降速、提速一次（代码里没有说明原因，只定义了这两个阶段的动作）。D80 的总线时钟不在状态机里改，沿用平台 FDT 配置的初始频率。

`StartApplication`：`DBG_START_APP_REQ` 命令，载荷是（`MAIN_ADDRESS`, 1）。bootrom 收到后把控制权交给应用固件。

`Stabilize`：等 `START_STABILIZE = 200ms`（`retry_at` 数据，由外界等）。固件接管后要初始化自己的 SDIO 接口。

`Reinitialize`：**重新执行 `VendorSetup` 序列**（`vendor_setup_operation(index, true)`）——固件接管后需要再做一遍厂商寄存器配置，且 `reinitialize=true` 时 V3 型号多写一条（`write_byte(0, 0x04, INTERRUPTS_ENABLED)`）。这解释了 `VendorSetup` 为什么设计成可重复的函数：同一序列被用两次，一次在固件上传前、一次在固件接管后。

`StackStart`：发 `MM_SET_STACK_START_REQ`（消息号 0x007b，目标 `TASK_MM`），载荷 `[1, 0, vendor, 0]`。这是 LMAC 协议栈的启动命令——它同时是**第一条第 3 章所说的"LMAC mailbox"命令**，标志通信从 bootrom 协议切换到应用固件协议。

`ArmChipInterrupt`：写芯片端 `interrupt_enable` 寄存器为 `INTERRUPTS_ENABLED`（DATA|COMMAND|ERROR 三位）。这是**芯片端**中断源的最后一枚开关。控制器侧的信号分两部分、时机不同：事务完成中断信号由 owner 启动时开（`AicOwner::start` 里的 `enable_completion_irq`，第 8 章）；CARD_INT 信号由每次 `rearm_and_check` 原子地开。物理 IRQ 的注册与使能则更早、在运行时 builder 完成（第 8 章）。

`Complete`：清掉 `startup`，状态转 `Ready`，发布 `AicEvent::Started { mac_address }`。

## 4.5 MAC 地址从哪来

`AicEvent::Started` 携带 `data.mac_address`，它的初值是 `[0; 6]`（`owner.rs` 构造时填入），当前代码中核心没有改写它的路径——启动完成后携带什么取决于构造时给的值。适配层收到 `Started` 后把它发布到 `MacAddressState`（一个 AtomicU64，`rdif/device/shared.rs:104`），此后 `NetControlEndpoint::mac_address` 读它即可，无需锁。发布通道是就绪的，值从哪来（固件配置区或平台提供）属于后续工作。

## 4.6 三件事撑起整个状态机

回顾全流程，只有三种元素在反复使用：

1. **一个 SDIO 请求 + 等回应**（Enable/SetBlockSize/读修订/读就绪）——用 `emit` 发出，回应推进 stage。
2. **一条 mailbox 命令**（所有 debug/LMAC 命令）——第 3 章的状态机，完成后 stage 前进。
3. **一个延时**（VendorDelay 5ms、Stabilize 200ms）——`retry_at` 数据，外界等待。

而驱动函数之间靠 `set_startup_stage` 移动指针。整个"启动一块 WiFi 芯片"的大事，被拆成 21 个阶段 × 每阶段至多一个 SDIO 请求的细粒度流水线，任何时刻都可以被取消（`Cancel` → `AbortSdio`，见第 7 章）、被超时（`NetOwnerStartup` 层的 deadline，见第 8 章）。

## 4.7 与旧实现的对比

旧启动（重构前 `fdrv/core/init.rs` 的 `init()`）是**一个大函数**：内部 sleep、内部轮询、内部发命令等回应，从头跑到尾，中间没有边界。新启动把它变成 21 个 stage，每个 stage 的推进函数只做"决定下一步发什么"这一件事。功能等价，但：

- 每次推进都在 owner 的控制下（第 8 章：运行时决定何时再调用）；
- 取消、超时、失败都有精确的插入点；
- 每个阶段可用假时间和假 SDIO 回应单独测试（`progress.rs` 末尾测试断言的就是阶段序列）。
