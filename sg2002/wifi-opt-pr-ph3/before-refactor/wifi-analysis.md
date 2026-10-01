# SG2002 StarryOS WiFi 驱动分析

**分支**：`sg2002/wifi`（基于 dev `759e69c1f`）

**日期**：2026-07-24

## 目录

- [1. 当前 WiFi 驱动实现状况](#1-当前-wifi-驱动实现状况)
- [2. 性能优化报告分析](#2-性能优化报告分析)
- [3. eBPF 驱动性能监测方案](#3-ebpf-驱动性能监测方案)

---

## 1. 当前 WiFi 驱动实现状况

### 1.1 整体架构

采用 TGOSKits 标准四层驱动模型：

| 层 | 组件 | 说明 |
|----|------|------|
| Driver Core | `components/aic8800/` (47 源文件) | OS 无关的 WiFi 芯片驱动 |
| Capability Boundary | `components/sdio-host/`, `components/sdhci-cv1800/` | SDIO 主机接口抽象 |
| OS Glue | `drivers/ax-driver/src/net/aic8800.rs`, `os/arceos/modules/axruntime/src/wifi_glue.rs` | MMIO 映射、IRQ 注册、运行时注入 |
| Control Plane | `net/ax-net/src/lib.rs` + `os/StarryOS/kernel/src/file/wext.rs` | `reconfigure_wifi()` + wireless-extensions ioctl |

### 1.2 核心组件

**AIC8800 WiFi Driver Core** (`components/aic8800/`)

| 模块 | 路径 | 说明 |
|------|------|------|
| 入口 | `src/lib.rs` | `probe`, `WifiRuntime`, `set_runtime` |
| 公共常量 | `src/common/mod.rs` | SDIO 寄存器地址, 芯片 VID/PID, 芯片变体 |
| 运行时注入 | `src/runtime.rs` | `WifiRuntime` trait：`now_nanos`, `sleep_ms`, `yield_now`, `spawn_poll_task`, `block_until` |
| 无线探测 | `src/wireless/mod.rs` | `probe()`：检测芯片变体, 加载固件, 启动 FDRV |
| 固件加载 | `src/fw/` | 固件下载, 芯片版本检测, IPC 协议 |
| FDRV 核心 | `src/fdrv/core/` | 总线管理, SDIO 传输, 初始化, PollSet |
| 协议层 | `src/fdrv/protocol/` | LMAC 命令, scan, connect, key install, APM, EAPOL |
| 加密 | `src/fdrv/crypto/wpa2.rs` | WPA2-PSK 四次握手 (PRF, AES-CCM, MIC) |
| 网络设备 | `src/fdrv/net/device.rs` | `AicWifiNetDev`：实现 `rd_net::Interface` 和 `rd_net::WifiControl` |
| 后台任务 | `src/fdrv/thread/` | TX/RX/AP 后台轮询任务 |
| API | `src/fdrv/wifi/api.rs` | `WifiClient`：scan, connect, disconnect, WPA2 握手, SoftAP |
| 连接管理 | `src/fdrv/wifi/manager.rs` | 连接状态管理, WPA2 IE 构造 |

**SDHCI Controller Driver** (`components/sdhci-cv1800/`)

| 文件 | 说明 |
|------|------|
| `src/lib.rs` | `CviSdhci`：SDHCI 命令处理 (CMD52/CMD53), PIO 数据传输, SDIO 卡枚举, 时钟设置, 4-bit bus |
| `src/hw_init.rs` | `sdio1_hw_init()`：Pinmux, CRG 时钟门控, RTC 域时钟, 复位, 卡检测 |
| `src/irq.rs` | 中断处理, CARD_INT 管理 |
| `src/regs.rs` | SDHCI 寄存器偏移和位定义 |
| `src/runtime.rs` | `SdhciDelay` trait (delay/yield 注入) |

**SDIO Host 抽象层**

- `components/sdio-host/`：`SdioHost` / `SdioCardIrq` trait
- `components/sdio-host2/`：新版 SD/SDIO/MMC 总线事务抽象

### 1.3 OS 集成

**WiFi 运行时胶水** (`os/arceos/modules/axruntime/src/wifi_glue.rs`)

`install_runtime()` 将 OS 无关的驱动核心接入 ArceOS：
- `WifiRuntime` → `ax_task::sleep` / `ax_hal::time::monotonic_time_nanos` / `ax_task::yield_now` / `ax_task::spawn_with_name`
- `SdhciDelay` → `ax_task::sleep`

**ax-driver AIC8800 绑定** (`drivers/ax-driver/src/net/aic8800.rs`)

启动流程：
1. MMIO 映射 SDIO1 控制器和 SoC 子系统寄存器
2. 解析 FDT "cvitek,cv181x-sdio" 兼容节点
3. 初始化 SDHCI 控制器
4. 探测 AIC8800 芯片
5. 启动 open SoftAP（"PicoClaw-Car", ch6, 192.168.50.1/24）
6. 附加 `WifiLinkPolicy`，注册为 `wlan0`
7. 注册 SDIO1 IRQ 处理

**内核 wireless-extensions** (`os/StarryOS/kernel/src/file/wext.rs`)

实现 "stage then commit" 模式：
- `SIOCSIWMODE` → 暂存模式
- `SIOCSIWESSID` → 暂存 SSID
- `SIOCSIWENCODEEXT` → 暂存密码
- `SIOCSIWFREQ` → 暂存频道
- `SIOCSIWCOMMIT` → 原子应用配置

**栈级 WiFi 控制** (`net/ax-net/src/lib.rs`)

- `register_wifi_control()`：按接口名存储 `WifiControlHandle`
- `WifiMode` 枚举：`Station` / `AccessPoint`
- `reconfigure_wifi()`：原子切换 STA/SoftAP，包含链路层和 IP/DHCP 角色切换

### 1.4 功能状态

| 功能 | 状态 |
|------|------|
| 芯片支持 | AIC8801, AIC8800DC, AIC8800D80, AIC8800D80X2 |
| SDIO 传输 | PIO 模式 (CMD52/CMD53), 4-bit bus |
| 固件加载 | 从上游获取, SHA-256 校验 |
| SoftAP 模式 | 开放网络, beacon, DHCP server (单客户端) |
| Station 模式 | scan, WPA2-PSK 四次握手 |
| WPA2-PSK | AES-CCM, PTK/GTK, EAPOL M1-M4 |
| 运行时 STA/AP 切换 | 通过 wireless-extensions ioctl |
| OOB RX 唤醒 | SDIO IRQ → wake net task |
| DHCP client | Station 模式工作 |
| 数据面 | `rd_net::Interface` + `rd_net::WifiControl` |
| WPA3 | 不支持 (有 `WifiAuthType::Wpa3Psk` 枚举预留) |
| cfg80211 | 无 (使用自研 wireless-extensions 子集) |
| 配置文件 | 无（启动时硬编码常量 + 运行时 ioctl） |

### 1.5 已知 TODO

1. `get_current_ssid()` — 从 bus 状态获取实际 SSID
2. `get_rssi()` — 信号强度查询
3. 连接断开后自动重连

### 1.6 WiFi 配置机制

**双机制，无文件配置**：

1. **启动时（编译期常量）**：`drivers/ax-driver/src/net/aic8800.rs:58-62`
   ```
   AP_SSID = "PicoClaw-Car"
   AP_CHANNEL = 6
   AP_SERVER_IP = 192.168.50.1/24
   ```
   改配置需要重新编译内核/驱动。

2. **运行时**：`wifi_switch` 工具通过 wireless-extensions ioctl 切换
   ```
   wifi_switch ap  <ssid> [channel]
   wifi_switch sta <ssid> [passphrase]
   ```

目标硬件：LicheeRV Nano / AKA-00 SG2002（Milk-V Duo 兼容）。

---

## 2. 性能优化报告分析

### 2.1 报告来源

`../sg2002-wifi.md`（StarOS WiFi 上行吞吐排查报告），平台 SG2002 "newboard"（cv1812cp）+ AIC8800DC WiFi over SDIO，分支 `test/verify-net-wakeup-fix`。

### 2.2 报告优化 vs 当前 `feat/net-enhance` 分支

报告中的优化**均未合入**当前 `feat/net-enhance` 分支：

**Part 1（根因修复，~0.2M → ~10M，50× 提升）**

| 优化项 | 报告修复 | `feat/net-enhance` 现状 |
|--------|---------|----------------------|
| `poll_int_status` 忙等策略 | Phase 1 时间上界 **3ms** 忙等 | Phase 1 固定 **1000 次**自旋 + Phase 2 `yield_now()` |

**Part 2（~10M → ~13.7M/18.9M，+37%/69× vs 起点）**

| # | 优化项 | 报告修复 | `feat/net-enhance` 现状 |
|---|--------|---------|----------------------|
| 1 | HT 结构体对齐 | `MAC_HT=32, MAC_HE=56, ME_CONFIG_REQ=112` | `26, 54, 102` |
| 2 | TX/RX kicker 周期 | 10ms → **1ms** | `sleep_ms(10)` |
| 3 | SDIO 50MHz + PHY delay | `50MHz` + `MSHC_CTRL\|=bit1` + `PHY_CONFIG\|=bit0` + `TX_RX_DLY=0x01000100` | `25MHz`，PHY 寄存器定义但未写入 |
| 4 | 流控空转修复 | `check_data_flow_control` 读1次 + yield | `for _ in 0..50` 空转重试 |
| 5 | 日志级别 | `Error` | `Info` |

### 2.3 根因概述

**Part 1 根因**：SDHCI PIO 传输中 `poll_int_status` 的 Phase 1 固定自旋窗口（~几十微秒）比硬件真实传输时间（~200µs）短，每笔 SDIO 写都掉入 Phase 2 的 `yield_now()`。一旦让出 CPU，sched-rr 的 50ms 时间片导致 TX 线程排队等满一个调度周期才被调度回来。一次亚毫秒的硬件等待被放大成 ~48ms。

**Part 2 根因**：
1. **HT 结构体对齐错误**：误用 packed 尺寸导致 ht_supp 写到错误偏移，固件读成 0 → 不通告 HT → 以 802.11g 关联 → 无 A-MPDU 聚合
2. **kicker 周期过长**：10ms kicker 在事件驱动不可靠时成为瓶颈
3. **SDIO 25MHz 封顶**：PHY delay 未配置导致 50MHz 下 DAT CRC error
4. **流控空转**：50MHz 下固件 buffer 被灌满，流控空转烧总线

### 2.4 整体提升曲线（报告数据）

| 里程碑 | 上行单路 | 提升 | vs Linux |
|--------|---------|------|----------|
| Part 1 起点 (legacy-g) | 0.2M | 1× | 0.6% |
| Part 1 终点 (SDHCI busy-wait) | ~10M | 50× | ~30% |
| + HT 对齐 + 1ms kicker | 12.7M | 64× | ~38% |
| + 50MHz SDIO + PHY + 修空转 | 13.7M | 69× | 41% |
| 目标 (追平 Linux) | — | — | 100% |

### 2.5 剩余差距拆解

| 根因 | 估计占比 | 证据 | 解法 |
|------|---------|------|------|
| HE vs HT (PHY 速率差) | ~60% | HT 65M PHY vs HE 115M PHY (1.77×) | 修 HE 数据通路 (ampdu=0) |
| 软件开销 (PIO vs DMA) | ~30% | TX busy 仅 12-14%, 固件排空受 PIO 限制 | DMA (ADMA2, 硬件 bit19=1) |
| 聚合深度差异 | ~10% | avg_ampdu 8-10× | DMA + 多帧拼包 |

---

## 3. eBPF 驱动性能监测方案

### 3.1 约束条件

- StarryOS 编译内联严重，直接 kretprobe 函数返回值不可靠（sret ABI 问题）
- 需避免异步结构（如 kprobe breakpoint 在调度路径上可能重入）
- SG2002 为 RISC-V 单核，无 BPF JIT，走 `rbpf` 解释器
- 目标：打点监测而非返回值拦截

### 3.2 推荐方案：静态 tracepoint + raw_tracepoint

**理由**：tracepoint 的所有限制是编译时确定的，运行时是 NOP 或直接函数调用，无 breakpoint 重入风险，payload 由驱动主动写入，不依赖寄存器/栈解析。

**现有基础设施**：

- `os/StarryOS/kernel/src/tracepoint/`：`ktracepoint` 子系统，已有 `sched_switch` 示例
- `ktracepoint::define_event_trace!` 宏：自动 link-section 注册
- 暴露到 debugfs：`/sys/kernel/debug/tracing/events/<category>/<event>/`
- BPF 挂载：通过 `BPF_RAW_TRACEPOINT_OPEN` 或 `perf_event_open(PERF_TYPE_TRACEPOINT, id)`

### 3.3 建议埋点

**SDHCI 总线层**

```
tracepoint: net, sdio_poll_complete
  dir: u8           // 0=TX, 1=RX
  nbytes: u32       // 本次传输字节数
  poll_us: u32      // poll_int_status 实际耗时 (us)
  phase2: bool      // 是否掉入 Phase 2 (yield 慢路径)
```

**WiFi 数据面**

```
tracepoint: net, wifi_tx_frame
  len: u32          // 帧长
  vif_idx: u8       // VIF 索引
  is_mgmt: bool     // 是否管理帧

tracepoint: net, wifi_flow_ctrl
  fc_value: u8      // 流控信用值
  blocked_ms: u32   // 流控等待时长 (ms)
```

### 3.4 关键监测指标

| 指标 | 埋点位置 | 诊断价值 |
|------|---------|---------|
| `poll_int_status` 耗时直方图 | `wait_transfer_complete` | 发现 SDIO 总线异常慢/卡死 |
| Phase 2 掉入率 | `poll_int_status` phase2 标志 | 发现忙等窗口不足、调度抖动 |
| PIO 每笔字节数分布 | `pio_read` / `pio_write` | 评估 DMA 迁移收益 |
| CMD53 块大小分布 | `cmd53_read_fixed` / `cmd53_write_fixed` | 评估多帧拼包收益 |
| A-MPDU 聚合深度 | `send_single_data_frame` 前后 | 验证 HE/HT 聚合是否生效 |
| 流控阻塞时长 | `check_data_flow_control` | 量化固件排空瓶颈 |
| TX/RX kicker 唤醒次数 | kicker 周期任务 | 判断事件驱动是否可靠 |

### 3.5 实现价值

1. **系统化诊断**：将报告中手工 `log::info!` 打点的排查过程固化为工具，无需反复编译上板
2. **长期回归防护**：未来 DMA/HE 等改动引入性能退化时可立即暴露
3. **量化优化优先级**：按实测数据分配 HE/PIO/聚合三类 gap 的投入
4. **零 I/O 开销**：ringbuf 走内存映射，不阻塞 UART

### 3.6 实现难度

| 工作项 | 难度 | 工作量 |
|--------|------|--------|
| 新增 tracepoint 定义 | 低 | ~50 行/事件 |
| 驱动热点埋点 | 低 | ~10 行/点 |
| eBPF 程序（内核侧） | 中 | ~200 行 |
| 用户态 loader | 低 | ~150 行 |
| 上板验证 | 中 | ~半天 |
| **合计** | 中 | **~2-3 天** |

### 3.7 备选方案：kprobe 入口探针

若不想改驱动代码，可给目标函数加 `#[inline(never)]`（已有 `net_stats` 先例），仅探入口参数：

```
kprobe:pio_write      → arg(2) = buf_len → 每笔 TX 字节数
kprobe:pio_read       → arg(2) = buf_len → 每笔 RX 字节数
kprobe:poll_int_status → bpf_ktime_get_ns() 记录 entry 时间戳
```

不探返回值（规避 sret），仅探入口参数。工作量约 1-2 天，代价是 `#[inline(never)]` 对热路径有轻微性能影响。
