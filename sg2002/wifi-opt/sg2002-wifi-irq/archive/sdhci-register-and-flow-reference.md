# SDHCI 寄存器快速对照表 & AIC8800 TX/RX 全流程对照图

> 基于当前 `sg2002/wifi-irq` 分支代码。每项标注了项目中的实现文件路径和行号。

---

## 一、SDHCI 寄存器快速对照表

> 所有寄存器常量定义在 `components/sdhci-cv1800/src/regs.rs`

### 1.1 标准 SDHCI 寄存器（SD Host Controller Spec v3.0）

| 偏移 | 名称 | 宽度 | 访问 | 用途 | 定义位置 |
|------|------|------|------|------|----------|
| `0x00` | DMA_ADDRESS | 32 | RW | ADMA2 描述符表物理地址（当前 PIO 模式，未使用） | `regs.rs:6` |
| `0x04` | BLOCK_SIZE | 16 | RW | CMD53 传输块大小（512B）+ SDMA boundary | `regs.rs:7` |
| `0x06` | BLOCK_COUNT | 16 | RW | CMD53 传输块数量 | `regs.rs:8` |
| `0x08` | ARGUMENT | 32 | RW | 命令参数（CMD52: func+addr+data; CMD53: 同上+byte count） | `regs.rs:9` |
| `0x0C` | TRANSFER_MODE | 16 | RW | 传输方向、多块、块计数使能 | `regs.rs:10` |
| `0x0E` | COMMAND | 16 | RW | 命令索引（CMD52=52, CMD53=53）+ 响应类型标志。写入触发执行 | `regs.rs:11` |
| `0x10` | RESPONSE | 32×4 | RO | 命令响应寄存器组（R5 = 仅 RESP[0] 有效） | `regs.rs:12` |
| `0x20` | BUFFER | 32 | RW | PIO 数据端口：逐 32-bit 读写 FIFO | `regs.rs:13` |
| `0x24` | PRESENT_STATE | 32 | RO | **总线状态**：CMD/DATA INHIBIT, BUF_WR/RD_EN, CARD_INSERTED | `regs.rs:14` |
| `0x28` | HOST_CONTROL | 8 | RW | 总线宽度(bit1)、高速使能(bit2)、卡检测 | `regs.rs:15` |
| `0x29` | POWER_CONTROL | 8 | RW | SD 总线电源：bit0=ON, bits[3:1]=3.3V → 写 `0x0F` | `regs.rs:16` |
| `0x2C` | CLOCK_CONTROL | 16 | RW | 内部时钟使能(bit0)、SD 时钟使能(bit2)、10-bit 分频器[15:6] | `regs.rs:17` |
| `0x2E` | TIMEOUT_CONTROL | 8 | RW | 数据超时计数器（写 `0x0E` = TMCLK × 2^27） | `regs.rs:18` |
| `0x2F` | SOFTWARE_RESET | 8 | W | 复位：bit0=ALL, bit1=CMD, bit2=DAT | `regs.rs:19` |

### 1.2 中断寄存器组（16-bit 分离访问）

| 偏移 | 名称 | 访问 | 用途 | 定义位置 |
|------|------|------|------|----------|
| `0x30` | INT_STATUS_NORM | R/W1C | **正常中断状态**：读当前状态，写 1 清除对应位 | `regs.rs:22` |
| `0x32` | INT_STATUS_ERR | R/W1C | **错误中断状态**：CMD/DAT timeout、CRC、end bit | `regs.rs:23` |
| `0x34` | NORM_INT_STS_EN | RW | 正常中断**状态**使能（置位后状态位才可见于 0x30） | `regs.rs:24` |
| `0x36` | ERR_INT_STS_EN | RW | 错误中断**状态**使能 | `regs.rs:25` |
| `0x38` | NORM_INT_SIG_EN | RW | 正常中断**信号**使能（置位后中断才上报到 PLIC） | `regs.rs:26` |
| `0x3A` | ERR_INT_SIG_EN | RW | 错误中断**信号**使能 | `regs.rs:27` |

**STS_EN vs SIG_EN 的区别**：STS_EN 控制状态寄存器是否可见该位（轮询用）；SIG_EN 控制是否把该位的中断信号发到 PLIC（ISR 用）。中断初始化见 `enable_irq_signals()` at `irq.rs:115`。

### 1.3 正常中断位定义（INT_STATUS_NORM, 16-bit）

| 位 | 常量 | 含义 | 触发时机 | 当前等待方式 | 定义位置 |
|----|------|------|---------|------------|----------|
| 0 | `CMD_COMPLETE` | 命令完成 | CMD52/CMD53 命令执行完毕 | Phase1 自旋 + Phase2 `delay_ms(10)` | `regs.rs:40` |
| 1 | `XFER_COMPLETE` | 传输完成 | CMD53 数据块全部传输完毕 | Phase1 自旋 + Phase2 **中断唤醒** | `regs.rs:41` |
| 4 | `BUF_WR_READY` | 写缓冲就绪 | FIFO 可接收下一块数据 | Phase1 自旋 + Phase2 `delay_ms(10)` | `regs.rs:42` |
| 5 | `BUF_RD_READY` | 读缓冲就绪 | FIFO 有数据可读 | Phase1 自旋 + Phase2 `delay_ms(10)` | `regs.rs:43` |
| 8 | `CARD_INT` | 卡中断 | WiFi 模组有数据/事件通知 CPU | ISR 处理 → mask + 回调 | `regs.rs:44` |
| 15 | `ERROR` | 错误汇总 | 任一错误中断位置位 | `poll_status_once` 中检测 | `regs.rs:45` |

### 1.4 错误中断位定义（INT_STATUS_ERR, 16-bit）

| 位 | 常量 | 含义 | 定义位置 |
|----|------|------|----------|
| 0 | `CMD_TIMEOUT` | 命令无响应超时 | `regs.rs:48` |
| 1 | `CMD_CRC` | 命令响应 CRC 错误 | `regs.rs:49` |
| 2 | `CMD_END_BIT` | 命令响应结束位错误 | `regs.rs:50` |
| 3 | `CMD_INDEX` | 命令响应索引不匹配 | `regs.rs:51` |
| 4 | `DAT_TIMEOUT` | 数据超时 | `regs.rs:52` |
| 5 | `DAT_CRC` | 数据 CRC 错误 | `regs.rs:53` |
| 6 | `DAT_END_BIT` | 数据结束位错误 | `regs.rs:54` |

### 1.5 中断信号使能策略（NORM_INT_SIG_EN）

> `enable_irq_signals()` 初始化写入 at `irq.rs:115-122`；动态 un-mask 在 `poll_int_status` at `lib.rs:191`；ISR 侧 mask 在 `irq.rs:209-217`。

| 位 | 初始化 | 动态行为 |
|----|--------|---------|
| `CARD_INT` | ✅ 使能 | 固定使能——`NORM_INT_SIG_MASK` 唯一置位项 (`regs.rs:78`) |
| `CMD_COMPLETE` | ✅ 使能 (STS_EN) | SIG_EN 始终使能，ISR 不处理——仅用于 `poll_int_status` 轮询 |
| `BUF_WR_READY` | ✅ 使能 (STS_EN) | SIG_EN 始终使能，仅用于轮询 |
| `BUF_RD_READY` | ✅ 使能 (STS_EN) | SIG_EN 始终使能，仅用于轮询 |
| `XFER_COMPLETE` | ❌ 初始不使能 | **动态**：`unmask_xfer_complete_signal()` at `irq.rs:154`；ISR 收到后立即 mask at `irq.rs:212` |
| `ERROR` | ✅ 使能 | 固定使能 |

### 1.6 传输模式 & 命令寄存器位

| 位 | 常量 | 含义 | 定义位置 |
|----|------|------|----------|
| 1 | `TM_BLK_CNT_EN` | 使能块计数（CMD53 必须置位） | `regs.rs:83` |
| 4 | `TM_DATA_DIR_READ` | 读方向（0=写, 1=读） | `regs.rs:84` |
| 5 | `TM_MULTI_BLOCK` | 多块传输（>1 block 时置位） | `regs.rs:85` |

| 响应类型 | 常量 | 用途 | 定义位置 |
|---------|------|------|----------|
| R4 (无CRC) | `CMD_FLAGS_R4` | CMD5 | `regs.rs:133` |
| R5 (48+CRC) | `CMD_FLAGS_R5` | CMD3, CMD52 | `regs.rs:135` |
| R1b (48+busy) | `CMD_FLAGS_R1B` | CMD7 | `regs.rs:137` |
| R5+Data | `CMD_FLAGS_R5_DATA` | CMD53 | `regs.rs:139` |

命令索引左移位数：`CMD_INDEX_SHIFT = 8` (`regs.rs:118`)。TRANSFER_MODE 与 COMMAND 作 32-bit 原子写入的逻辑见 `cmd53_xfer()` at `lib.rs:448-449`。

### 1.7 Vendor 寄存器（CV1800/SG2002 专用）

| 偏移 | 名称 | 用途 | 定义位置 |
|------|------|------|----------|
| `0x200` | VENDOR_MSHC_CTRL | SDIO 控制器杂项控制：反馈时钟(bit1)、TX/RX delay 使能(bit8/9)、SD1_SEL(bit16) | `regs.rs:159` |
| `0x240` | VENDOR_PHY_TX_RX_DLY | PHY 收发时序延迟校准 | `regs.rs:160` |
| `0x24C` | VENDOR_PHY_CONFIG | PHY 配置使能(bit0) | `regs.rs:161` |

Vendor 位定义：`VENDOR_MSHC_CTRL_FEEDBACK_CLK` (`regs.rs:164`)、`VENDOR_MSHC_CTRL_TX_DLY_EN` (`regs.rs:165`)、`VENDOR_MSHC_CTRL_RX_DLY_EN` (`regs.rs:166`)、`VENDOR_MSHC_CTRL_SD1_SEL` (`regs.rs:167`)、`VENDOR_PHY_ENABLE` (`regs.rs:170`)。

PHY delay 的初始化逻辑在 `hw_init.rs` 的 `apply_vendor_phy_delay()` 中（当前分支写 0，Part 2 分支写精确序列）。

### 1.8 Present State 寄存器位（0x24）

| 位 | 常量 | 含义 | 定义位置 | 读取位置 |
|----|------|------|----------|----------|
| 0 | `CMD_INHIBIT` | CMD 线忙 | `regs.rs:33` | `wait_cmd_idle()` at `lib.rs:242-250` |
| 1 | `DATA_INHIBIT` | DAT 线忙 | `regs.rs:34` | `wait_data_idle()` at `lib.rs:253-265` |
| 10 | `BUF_WR_EN` | 写缓冲可用 | `regs.rs:35` | — |
| 11 | `BUF_RD_EN` | 读缓冲可用 | `regs.rs:36` | — |
| 16 | `CARD_INSERTED` | 卡在位 | `regs.rs:37` | — |

### 1.9 时钟控制寄存器位（0x2C）

| 位 | 常量 | 含义 | 定义位置 |
|----|------|------|----------|
| 0 | `CC_INT_CLK_EN` | 内部时钟使能 | `regs.rs:92` |
| 1 | `CC_INT_CLK_STABLE` | 内部时钟稳定标志（RO） | `regs.rs:93` |
| 2 | `CC_SD_CLK_EN` | SD 时钟输出使能 | `regs.rs:94` |
| [7:6] | `CC_FREQ_SEL_EXT_MASK` | 分频器高 2 位（10-bit 模式） | `regs.rs:95` |
| [15:8] | `CC_FREQ_SEL_MASK` | 分频器低 8 位 | `regs.rs:96` |

时钟配置逻辑：`configure_clock()` at `lib.rs:286-347`，分频器计算使用 `DIV_FACTOR=2` (`regs.rs:153`)、`CVI_SDIO_SRC_CLOCK_HZ=375_000_000` (`regs.rs:149`)。

### 1.10 复位 & 电源 & Host Control

| 寄存器 | 常量 | 含义 | 定义位置 |
|--------|------|------|----------|
| SOFTWARE_RESET (0x2F) | `SWRST_ALL` (bit0) | 全复位 | `regs.rs:101` |
| | `SWRST_CMD_LINE` (bit1) | CMD 线复位 | `regs.rs:102` |
| | `SWRST_DAT_LINE` (bit2) | DAT 线复位 | `regs.rs:103` |
| POWER_CONTROL (0x29) | `POWER_330V_ON` = `0x0F` | 3.3V 上电 | `regs.rs:108` |
| HOST_CONTROL (0x28) | `HC_BUS_WIDTH_4` (bit1) | 4-bit 总线模式 | `regs.rs:111` |
| | `HC_HIGH_SPEED` (bit2) | 高速模式使能 | `regs.rs:112` |

复位流程：`reset_controller()` at `lib.rs:276-285`；`reset_dat_line()` at `lib.rs:126-130`。

---

## 二、TX 写入全流程（应用层 → SDIO 硬件）

### 2.1 流程总览

```
Layer 4: 应用层         iperf3 / akars
                           │ socket write()
                           ▼
                       ┌─────────────────────────────────────────┐
Layer 3: TCP/IP        │ smoltcp TCP socket buffer (64KB)        │
                       │ Service::poll()                        │ ← net/ax-net/src/service.rs:803
                       │   → iface.poll() → Router::dispatch()   │ ← net/ax-net/src/router.rs:899
                       └─────────────────────────────────────────┘
                           │
                           ▼
                       ┌─────────────────────────────────────────┐
Layer 2: 网络设备      │ DeviceHandle::enqueue_tx()             │ ← net/ax-net/src/router.rs:439
                       │   Copy 1: IP packet → QueuedPacket      │
                       │   ([u8; 1500] 栈内)                     │
                       └─────────────────────────────────────────┘
                           │ wake device TX worker
                           ▼
                       ┌─────────────────────────────────────────┐
                       │ device_tx_worker()                     │ ← net/ax-net/src/router.rs:1064
                       │   Copy 2: QueuedPacket → Ethernet Vec<u8>│
                       │   device.send() → send_to()             │
                       └─────────────────────────────────────────┘
                           │
                           ▼
                       ┌─────────────────────────────────────────┐
                       │ RdNetDriver::transmit()                 │
                       │   Copy 3: Vec<u8> → DMA buffer          │
                       └─────────────────────────────────────────┘
                           │
                           ▼
                       ┌─────────────────────────────────────────┐
                       │ AicTxQueue::submit()                    │ ← components/aic8800/src/fdrv/net/device.rs:232
                       │   Copy 4: DMA buffer → Vec<u8> (.to_vec)│
                       │   → enqueue_data_frame()                │ ← components/aic8800/src/fdrv/thread/tx.rs:548
                       │     入队 bus.tx.queue (MAX 256 槽)       │   consts.rs:292; 检查 logic at tx.rs:550
                       │     wake tx poll task                    │
                       └─────────────────────────────────────────┘
                           │
                           ▼
                       ╔═════════════════════════════════════════╗
Layer 1: WiFi 驱动     ║ wifi-tx poll task: tx_process()        ║ ← tx.rs:517
                       ║   │                                    ║
                       ║   ├─ CMD pending? → 中断 DATA 批次     ║ ← tx.rs:228 (bus.cmd.pending_flag check)
                       ║   │                                    ║
                       ║   └─ process_data_tx()                 ║ ← tx.rs:207
                       ║        │ batch_count < TX_BATCH_LIMIT   ║    consts.rs:289 (=64); 检查 at tx.rs:229
                       ║        │                               ║
                       ║        └─ send_single_data_frame()     ║ ← tx.rs:271
                       ║             │                          ║
                       ║             ├─ ① build_data_frame()    ║ ← tx.rs:350
                       ║             │     Vec<u8> 堆分配 (~1536B)║
                       ║             │     Copy 5: eth_frame→SDIO║
                       ║             │     + fill_hostdesc()     ║ ← tx.rs:414
                       ║             │                          ║
                       ║             ├─ ② check_data_flow_ctrl() ║ ← tx.rs:255
                       ║             │     CMD52 读固件 credit   ║
                       ║             │     credits ≤ 2 → 阻塞    ║    DATA_FLOW_CTRL_THRESH=2 at consts.rs:295
                       ║             │     最多 50× 重试 + yield ║    (代码中 hardcode 50, FLOW_CONTROL_MAX_RETRY=100 未用)
                       ║             │                          ║
                       ║             └─ ③ transport.write_fifo()║ ← sdio_transport.rs:166
                       ║                  sdio.lock() ← 全局锁  ║    SdioTransport.sdio at sdio_transport.rs:29
                       ╚═════════════════════════════════════════╝
                           │
                           ▼
                       ╔═════════════════════════════════════════╗
Layer 0: SDHCI 硬件     ║ CviSdhci::write_fifo()                ║ ← components/sdhci-cv1800/src/lib.rs (SdioHost impl)
                       ║   │                                    ║
                       ║   └─ cmd53_write_fixed()               ║ ← lib.rs:468
                       ║        │                               ║
                       ║        ├─ ④ cmd53_xfer()               ║ ← lib.rs:383
                       ║        │     ├─ wait_cmd_idle()         ║ ← lib.rs:242   (PRESENT_STATE, 纯自旋)
                       ║        │     ├─ wait_data_idle()        ║ ← lib.rs:253   (PRESENT_STATE, 纯自旋)
                       ║        │     ├─ write BLOCK_SIZE/COUNT/ARG                    lib.rs:423-427, 435-437
                       ║        │     ├─ write TRANSFER_MODE|COMMAND (32-bit atomic)    lib.rs:448-449
                       ║        │     └─ wait_cmd_complete()     ║ ← lib.rs:224 → poll_int_status(CMD_COMPLETE)
                       ║        │           Phase1: 1000× 自旋   ║    PHASE1_SPIN_ITERS=1000 at lib.rs:33
                       ║        │           Phase2: delay_ms(10) ║    lib.rs:196
                       ║        │                               ║
                       ║        ├─ ⑤ pio_write(buf,512,nblocks) ║ ← lib.rs:503
                       ║        │     for each block:            ║
                       ║        │       ├─ wait_buf_wr_ready()   ║ ← lib.rs:233 → poll_int_status(BUF_WR_READY)
                       ║        │       │   Phase2: delay_ms(10) ║    ★ 非中断路径
                       ║        │       └─ 128× write::<u32>    ║    lib.rs:516 (SDHCI_BUFFER=0x20)
                       ║        │                               ║
                       ║        └─ ⑥ wait_transfer_complete()   ║ ← lib.rs:237 → poll_int_status(XFER_COMPLETE)
                       ║              Phase1: 1000× 自旋        ║
                       ║              Phase2: unmask SIG_EN      ║ ← irq.rs:154 (unmask_xfer_complete_signal)
                       ║                       block_timeout    ║ ← runtime.rs:19 (WaitQueue 中断唤醒)
                       ║                       ISR mask SIG_EN  ║ ← irq.rs:212 (rmw_norm_sig_en)
                       ╚═════════════════════════════════════════╝
                           │
                           └─ sdio.unlock()
```

### 2.2 等待点汇总

| 步骤 | 函数 | 等待位 | Phase 1 | Phase 2 | 调用频率 | 实现位置 |
|------|------|--------|---------|---------|---------|----------|
| ④ cmd_complete | `wait_cmd_complete()` | `CMD_COMPLETE` | 1000× 自旋 | 10ms 睡眠 × 20 | 每帧 1 次 | `lib.rs:224` → `lib.rs:157` |
| ⑤ buf_wr_ready | `wait_buffer_write_ready()` | `BUF_WR_READY` | 1000× 自旋 | 10ms 睡眠 × 20 | **每 block 1 次** | `lib.rs:233` → `lib.rs:157` |
| ⑥ xfer_complete | `wait_transfer_complete()` | `XFER_COMPLETE` | 1000× 自旋 | 10ms 中断唤醒 × 20 | 每帧 1 次 | `lib.rs:237` → `lib.rs:157` |
| data_idle | `wait_data_idle()` | —（PRESENT_STATE） | 100,000× 自旋 | 无 Phase2 | 每帧 1 次 | `lib.rs:253` |
| cmd_idle | `wait_cmd_idle()` | —（PRESENT_STATE） | 100,000× 自旋 | 无 Phase2 | 每帧 1 次 | `lib.rs:242` |

Phase 常量：`PHASE1_SPIN_ITERS=1000` (`lib.rs:33`)、`PHASE2_STEP_MS=10` (`lib.rs:35`)、`PHASE2_MAX_ITERS=20` (`lib.rs:37`)、`PHASE2_WARN_AT=10` (`lib.rs:39`)。

`poll_int_status` 主体 at `lib.rs:157-222`；`poll_status_once` at `lib.rs:132-148`；`clear_int_status_norm` at `lib.rs:123-130`。

### 2.3 TX 路径拷贝统计

| # | 阶段 | 从 | 到 | 分配 | 位置 |
|---|------|-----|-----|------|------|
| 1 | Router dispatch | smoltcp tx_buffer | `QueuedPacket` ([u8; 1500]) | 栈 | `net/ax-net/src/router.rs:439` |
| 2 | EthernetDevice::send_to | `QueuedPacket` | `Vec<u8>` | 堆 | `net/ax-net/src/router.rs` (`DeviceHandle::enqueue_tx` 下游) |
| 3 | RdNetDriver::transmit | `Vec<u8>` | DMA buffer | 池 | `drivers/ax-driver` (RdNetDriver impl) |
| 4 | AicTxQueue::submit | DMA buffer | `Vec<u8>` (.to_vec()) | 堆 | `aic8800/src/fdrv/net/device.rs:232-244` |
| 5 | build_data_frame | eth_frame | SDIO frame ~1536B | 堆 | `aic8800/src/fdrv/thread/tx.rs:350` |

**每帧 5 次全帧拷贝 + 3 次堆分配。**

---

## 三、RX 读取全流程（CARD_INT → 应用层）

### 3.1 流程总览

```
Layer 0: 硬件中断       WiFi 模组有数据
                           │ 拉 CARD_INT 引脚 → SDHCI 控制器
                           │ INT_STATUS_NORM bit8 置位
                           │ SIG_EN bit8=1 → 上报 PLIC IRQ#38
                           ▼
                       ╔═════════════════════════════════════════╗
                       ║ sdhci_irq_handler()                    ║ ← irq.rs:177
                       ║   │                                    ║
                       ║   ├─ 读 INT_STATUS_NORM               ║    irq.rs:185
                       ║   ├─ status==0 → return (spurious)     ║    irq.rs:186-188
                       ║   │                                    ║
                       ║   ├─ CARD_INT?                        ║    irq.rs:193
                       ║   │   ├─ mask_card_irq_raw(base,true)  ║    irq.rs:195 → irq.rs:163
                       ║   │   │   (SIG_EN RMW: clear bit8)     ║
                       ║   │   └─ card_irq_callback.invoke()    ║    irq.rs:198
                       ║   │       │                            ║
                       ║   │       ▼                            ║
                       ║   │     sdio1_irq_handler()            ║ ← aic8800/src/fdrv/core/bus.rs:316
                       ║   │       mask CARD_INT (aic 侧)       ║    bus.rs:322
                       ║   │       bus.rx.irq_waker.wake()      ║    bus.rs:325
                       ║   │       wake wifi-rx poll task        ║
                       ║   │                                    ║
                       ║   └─ XFER_COMPLETE?                    ║    irq.rs:210
                       ║       ├─ rmw_norm_sig_en (mask SIG_EN) ║    irq.rs:212
                       ║       └─ pio_wake_callback.invoke()     ║    irq.rs:216
                       ║           → WaitQueue::notify_all()     ║    wifi_glue.rs:70-73
                       ╚═════════════════════════════════════════╝
                           ▼
                       ╔═════════════════════════════════════════╗
Layer 1: WiFi 驱动     ║ wifi-rx poll task 被唤醒               ║
                       ║   │  (idle 时 sleep_ms(10) 兜底)       ║    rx.rs:132
                       ║   │                                    ║
                       ║   └─ process_rx_frames()               ║ ← rx.rs:273
                       ║        │                               ║
                       ║        ├─ ① mask_card_irq() 防重入     ║    sdio_transport.rs:181
                       ║        │                               ║
                       ║        ├─ ② drain func2 (CMD52 读 blk) ║    rx.rs:288 drain_func()
                       ║        │     if blocks>0:               ║
                       ║        │       read_fifo_data(func2)    ║ ← rx.rs:242
                       ║        │       解析 CFM / indication    ║
                       ║        │                               ║
                       ║        ├─ ③ drain func1 (CMD52 读 blk) ║
                       ║        │     if blocks>0:               ║
                       ║        │       read_fifo_data(func1)    ║ ← rx.rs:242
                       ║        │         │                     ║
                       ║        │         │ while offset < len:  ║    rx.rs:246
                       ║        │         │   chunk=min(rem, 512)║    SDIOWIFI_FUNC_BLOCKSIZE=512 at consts.rs:12
                       ║        │         │   transport          ║
                       ║        │         │     .read_fifo()     ║ ← sdio_transport.rs:156
                       ║        │         │     sdio.lock()      ║
                       ║        │         │       │              ║
                       ║        │         │       ▼              ║
                       ║        │         │     CviSdhci::       ║
                       ║        │         │       read_fifo()    ║ ← lib.rs (SdioHost impl)
                       ║        │         │       └─ cmd53_     ║
                       ║        │         │          read_fixed()║ ← lib.rs:455
                       ║        │         │          ├─ cmd53_  ║
                       ║        │         │          │  xfer()   ║ ← lib.rs:383
                       ║        │         │          │  (同上TX) ║
                       ║        │         │          ├─ pio_read ║ ← lib.rs:482
                       ║        │         │          │ per blk:  ║
                       ║        │         │          │ wait_buf_ ║
                       ║        │         │          │ rd_ready  ║ ← lib.rs:229 → poll_int_status(BUF_RD_READY)
                       ║        │         │          │ 128× read ║    lib.rs:490 (SDHCI_BUFFER)
                       ║        │         │          └─ wait_   ║
                       ║        │         │             xfer_    ║
                       ║        │         │             complete ║ ← lib.rs:237
                       ║        │         │     sdio.unlock()    ║
                       ║        │         │                     ║
                       ║        │         └─ Copy 1: SDIO FIFO  ║
                       ║        │              → Vec<u8>  堆分配 ║    rx.rs:243
                       ║        │                               ║
                       ║        │   parse 802.11 frame → dispatch║
                       ║        │     ├─ Data frame:            ║
                       ║        │     │   build_and_enqueue_     ║
                       ║        │     │     eth_frame()          ║ ← rx.rs:594
                       ║        │     │     Copy 2: 802.11 MPDU  ║
                       ║        │     │       → Ethernet Vec<u8> ║
                       ║        │     │     → bus.rx.data_queue  ║
                       ║        │     │     → invoke_rx_data_    ║
                       ║        │     │       callback()         ║ → wake net-poll worker
                       ║        │     │                         ║
                       ║        │     └─ CFM/Indication:        ║
                       ║        │         → bus.cmd.cfm_queue    ║
                       ║        │           / ind_queue          ║
                       ║        │         → wake cmd poll task   ║
                       ║        │                               ║
                       ║        └─ unmask_card_irq() 恢复 CARD  ║    sdio_transport.rs:188
                       ╚═════════════════════════════════════════╝
                           │
                           ▼
                       ┌─────────────────────────────────────────┐
Layer 2/3: 网络栈     │ net-poll worker 被唤醒                  │
                       │                                        │
                       │ device_rx_worker()                     │ ← net/ax-net/src/router.rs:1089
                       │   Copy 3: data_queue → DMA buffer      │
                       │                                        │
                       │ RdNetDriver::receive()                 │
                       │   → prefetch_rx_packets()               │
                       │   Copy 4: DMA buffer → VecRxBuffer     │
                       │                                        │
                       │ EthernetDevice::recv()                  │ ← net/ax-net/src/router.rs:1309
                       │   Copy 5: payload → DevicePacketBuffer  │
                       │                                        │
                       │ Router RX enqueue                      │
                       │   Copy 6: DevicePacketBuffer            │
                       │          → QueuedPacket                 │
                       │                                        │
                       │ Router::poll() → smoltcp                │
                       │   Copy 7: QueuedPacket → rx_buffer      │
                       └─────────────────────────────────────────┘
                           ▼
Layer 4: 应用层          iperf3 / akars recv()
```

### 3.2 RX 路径拷贝统计

| # | 阶段 | 从 | 到 | 位置 |
|---|------|-----|-----|------|
| 1 | read_fifo_data | SDIO FIFO | `Vec<u8>` | `aic8800/src/fdrv/thread/rx.rs:242-267` |
| 2 | build_and_enqueue_eth_frame | 802.11 MPDU | Ethernet `Vec<u8>` | `aic8800/src/fdrv/thread/rx.rs:594` |
| 3 | device_rx_worker | data_queue | DMA buffer | `net/ax-net/src/router.rs:1089` |
| 4 | RdNetDriver::prefetch | DMA buffer | VecRxBuffer (.to_vec) | `drivers/ax-driver` |
| 5 | EthernetDevice::recv | payload | DevicePacketBuffer | `net/ax-net/src/router.rs:1309` |
| 6 | Router RX enqueue | DevicePacketBuffer | QueuedPacket | `net/ax-net/src/router.rs` |
| 7 | Router::poll | QueuedPacket | smoltcp rx_buffer | `net/ax-net/src/router.rs:899` |

**每帧 7 次全帧拷贝。**

---

## 四、TX vs RX 架构差异总结

| 维度 | TX（上传） | RX（下载） |
|------|-----------|-----------|
| **启动方式** | poll task 拉取队列 (`tx.rs:517`) | CARD_INT 硬件中断 (`irq.rs:193`) → wake poll task (`bus.rs:325`) |
| **poll 兜底** | 空闲时 `sleep_ms(10)` (`tx.rs:107`) | 空闲时 `sleep_ms(10)` (`rx.rs:132`) |
| **数据方向** | 推：CPU → SDIO → 模组 | 拉：模组 → SDIO → CPU |
| **流控** | 固件 credit 反压（≤2 即阻塞）(`tx.rs:255-268`) | 无（CPU 主动读，读走即腾出 buffer） |
| **SDIO 锁** | `Arc<Mutex<dyn SdioHost>>` 持锁写 (`sdio_transport.rs:29`) | 同锁，与 TX 互斥 |
| **关键等待** | CMD_COMPLETE + BUF_WR_READY(×N blocks) + XFER_COMPLETE | CMD_COMPLETE + BUF_RD_READY(×N blocks) + XFER_COMPLETE |
| **中断覆盖** | XFER_COMPLETE ✅ | CMD_COMPLETE / BUF_WR / BUF_RD ❌ | 同左 |
| **入口函数** | `cmd53_write_fixed()` (`lib.rs:468`) | `cmd53_read_fixed()` (`lib.rs:455`) |

---

## 五、中断与回调注册链路

### 5.1 ISR 注册链路

```
PLIC IRQ#38
  └─ sdhci_irq_handler()              ← irq.rs:177
       │
       ├─ CARD_INT → card_irq_callback.invoke()
       │                │
       │                └─ sdio1_irq_handler()   ← aic8800/src/fdrv/core/bus.rs:316
       │                     注册: register_card_irq_callback(sdio1_irq_handler)
       │                           at aic8800/src/wireless/mod.rs:62
       │                     定义: irq.rs:101-103
       │
       └─ XFER_COMPLETE → pio_wake_callback.invoke()
                            │
                            └─ sdhci_pio_wake_callback()  ← os/arceos/modules/axruntime/src/wifi_glue.rs:70
                                 注册: register_pio_wake_callback(sdhci_pio_wake_callback)
                                       at wifi_glue.rs:101
                                 定义: irq.rs:110-112
```

### 5.2 CallbackSlot 机制

`CallbackSlot` struct at `irq.rs:27-64`：
- `new()` at `irq.rs:32` — AtomicUsize 初始化为 0
- `register(cb)` at `irq.rs:39` — `cb as usize` 以 Release 语义存储
- `invoke()` at `irq.rs:55` — Acquire 加载，零值守卫，`transmute::<usize, fn()>(v)` 调用

`SdhciIrqState` at `irq.rs:67-84`：持有一个 `base`、一个 `card_irq_callback`、一个 `pio_wake_callback`。

### 5.3 SIG_EN RMW 辅助函数

`rmw_norm_sig_en(base, set, clear)` at `irq.rs:144-148`：
- 读 `SDHCI_NORM_INT_SIG_EN` → 清除 `clear` 位 → 置位 `set` 位 → 写回
- 非原子（可被 ISR 抢占），靠 XFER_COMPLETE sticky bit 自愈
- `unmask_xfer_complete_signal()` at `irq.rs:154-160` 是唯一调用者（set XFER_COMPLETE, clear 0）
- `mask_card_irq_raw()` at `irq.rs:163-169`：mask=false → set CARD_INT；mask=true → clear CARD_INT

---

## 六、关键常量速查

| 常量 | 值 | 含义 | 定义位置 |
|------|-----|------|----------|
| `PHASE1_SPIN_ITERS` | 1000 | Phase 1 自旋次数 (~50µs) | `components/sdhci-cv1800/src/lib.rs:33` |
| `PHASE2_STEP_MS` | 10 | Phase 2 每步延迟 ms | `components/sdhci-cv1800/src/lib.rs:35` |
| `PHASE2_MAX_ITERS` | 20 | Phase 2 最大迭代次数 (总计 200ms) | `components/sdhci-cv1800/src/lib.rs:37` |
| `PHASE2_WARN_AT` | 10 | 第 10 次迭代时打 warn | `components/sdhci-cv1800/src/lib.rs:39` |
| `SDIOWIFI_FUNC_BLOCKSIZE` | 512 | SDIO 功能块大小 (字节) | `components/aic8800/src/fdrv/consts.rs:12` |
| `FLOW_CONTROL_MAX_RETRY` | 100 | 流控重试上限（当前分支未用，硬编码 50） | `components/aic8800/src/fdrv/consts.rs:28` |
| `TX_BATCH_LIMIT` | 64 | 单次 process_data_tx 最大批处理帧数 | `components/aic8800/src/fdrv/consts.rs:289` |
| `MAX_TX_QUEUE_LEN` | 256 | TX 帧队列容量上限 | `components/aic8800/src/fdrv/consts.rs:292` |
| `DATA_FLOW_CTRL_THRESH` | 2 | 流控 credit 阈值 (≤2 即停止发送) | `components/aic8800/src/fdrv/consts.rs:295` |
| `CONTROL_PORT_RECONCILE_MS` | 50 | AP 控制端口对账周期 | `components/aic8800/src/fdrv/consts.rs:64` |
| `MAX_REGISTERED_STAS` | 16 | AP 模式注册表容量 | `components/aic8800/src/fdrv/consts.rs:56` |
| `HIGH_SPEED_CLOCK_HZ` | 25_000_000 | 当前 SDIO 时钟 | `components/sdhci-cv1800/src/regs.rs:145` |
| `CVI_SDIO_SRC_CLOCK_HZ` | 375_000_000 | SoC SDIO 源时钟 | `components/sdhci-cv1800/src/regs.rs:149` |
| `CMD_RESPONSE_TIMEOUT` | 100_000 | wait_data_idle/wait_cmd_idle 自旋上限 | `components/sdhci-cv1800/src/regs.rs:179` |
| `RESET_TIMEOUT` | 100_000 | wait_reset_complete 自旋上限 | `components/sdhci-cv1800/src/regs.rs:177` |
| `SDHCI_SDMA_BOUNDARY_512K` | 0x7 << 12 | SDMA boundary (SD 规范要求) | `components/sdhci-cv1800/src/regs.rs:89` |
| `NORM_INT_SIG_MASK` | CARD_INT only | 初始 SIG_EN 使能掩码 | `components/sdhci-cv1800/src/regs.rs:78` |
| `NORM_INT_ENABLE_MASK` | CMD+XFER+BUF_WR+BUF_RD+CARD | 初始 STS_EN 使能掩码 | `components/sdhci-cv1800/src/regs.rs:58-62` |
| `WifiBus` struct | — | WiFi 总线状态（tx/rx/cmd queues, transport, irq_waker） | `components/aic8800/src/fdrv/core/bus.rs:210` |
| `SdioTransport` struct | — | SDIO 传输层（锁 + 地址 + 中断控制） | `components/aic8800/src/fdrv/core/sdio_transport.rs:27` |
| `ArceosDelay` struct + impl | — | ArceOS 侧 SdhciDelay 实现（WaitQueue 中断唤醒） | `os/arceos/modules/axruntime/src/wifi_glue.rs:75-87` |
| `debug_assert!(cpu_num() == 1)` | — | 单核假设断言 | `os/arceos/modules/axruntime/src/wifi_glue.rs:95-96` |

---

## 七、文件索引

| 文件 | 主要内容 |
|------|---------|
| `components/sdhci-cv1800/src/regs.rs` | 所有 SDHCI 寄存器偏移、位定义、时钟常量 |
| `components/sdhci-cv1800/src/lib.rs` | PIO 读写、CMD53 传输、poll_int_status、时钟配置、复位 |
| `components/sdhci-cv1800/src/irq.rs` | ISR、CallbackSlot、SIG_EN RMW、中断初始化 |
| `components/sdhci-cv1800/src/runtime.rs` | `SdhciDelay` trait（delay_ms + block_timeout） |
| `components/sdhci-cv1800/src/hw_init.rs` | SoC 级初始化（CRG/SYSCON/RTCSYS, PHY delay） |
| `components/aic8800/src/fdrv/thread/tx.rs` | TX 线程：帧构建、流控、CMD53 写入、enqueue |
| `components/aic8800/src/fdrv/thread/rx.rs` | RX 线程：FIFO 读取、帧解析、分发 |
| `components/aic8800/src/fdrv/core/sdio_transport.rs` | SDIO 操作封装（read/write_fifo, flow_ctrl）、SdioTransport |
| `components/aic8800/src/fdrv/core/bus.rs` | WifiBus、ISR 回调 sdio1_irq_handler、irq_waker |
| `components/aic8800/src/fdrv/net/device.rs` | ITxQueue/IRxQueue 实现（AicTxQueue/AicRxQueue） |
| `components/aic8800/src/fdrv/consts.rs` | 驱动常量：块大小、流控、队列容量、超时 |
| `os/arceos/modules/axruntime/src/wifi_glue.rs` | ArceOS 胶水层：ArceosDelay、PIO wake callback、单核断言 |
| `net/ax-net/src/router.rs` | 网络栈路由：DeviceHandle、device_tx/rx_worker、EthernetDevice |
| `net/ax-net/src/service.rs` | Service::poll() → smoltcp iface.poll() |
