# iperf3 网络性能对比测试 — Linux vs StarryOS（LicheeRV Nano / SG2002）

> **重要说明（标注）**：本文档**所有测试结果**，均为 **Windows(PC) 端 iperf3（Cygwin 3.17.1）作为测试一端**、板端 iperf3 作为另一端测得。
> 即：`board-s` = 板端作 server、**Windows 端 iperf3 作 client**；`board-c` = 板端作 client、**Windows 端 iperf3 作 server**。
> 因此所有数字都受 Windows/Cygwin 端影响（见 §3 工具限制），**不代表板端单独能力上限**；若需要板端纯能力，应改用 Linux↔Linux 或板端 loopback 复测。

## 1. 测试环境

| 项 | 内容 |
|---|---|
| 开发板 | LicheeRV Nano (SG2002)，CPU isa `rv64imafdvcsu`，WiFi 芯片日志前缀 `AICWFDBG` |
| 板端 OS（Linux） | Buildroot 2023.11.2，kernel `5.10.4-tag-` riscv64，**musl** libc |
| 板端 iperf3（Linux） | **3.14**（预装 `/usr/bin/iperf3`，musl 动态链接） |
| 板端 OS（StarryOS） | `uname` 报告 `Linux starry 10.0.0 riscv64`；内核日志前缀 `starry_kernel::*`，网络栈 `ax_net::*` |
| 板端 iperf3（StarryOS） | **3.19.1** |
| PC | Windows 11 (22621) |
| PC iperf3 | **3.17.1**（官方 Cygwin 构建，`tools\iperf3-win\iperf3.17.1_64\iperf3.exe`） |
| 测试链路 | WiFi：PC USB 网卡热点 `192.168.137.1` ↔ 板 `wlan0` STA `192.168.137.68` |
| 控制/记录 | COM5 串口 115200（root/root），纯串口控制；SSH 仅用于大文件传输，测试期间关闭 |
| 日志 | 原始日志见 `logs/`、`<os>/<mode>/cases/*.log`、`<os>/<mode>/board_serial.log`；结果汇总 `<os>/<mode>/cases.csv` |

## 2. 测试参数（最佳实践）

- 时长 `-t 30`，预热丢弃 `-O 3`，间隔 `-i 1`
- 单流 `-P 1` / 多流 `-P 4`
- TCP：默认窗口/拥塞控制
- UDP：`-u -b 0`（饱和，测容量与丢包/抖动）
- 方向定义（相对**板端**）：`tx` = 板→PC；`rx` = PC→板；`bidir` = 双向
- 角色：`board-s` = 板作 server（PC 作 client）；`board-c` = 板作 client（PC 作 server）
- 每侧均记录日志（客户端日志 + 服务端日志）

## 3. 工具限制与替代方法（重要）

1. **Windows 端 iperf3 均为 Cygwin 构建**，其 **server 无法处理 UDP `-P>1`**（`--bidir` 对 UDP 等于 2 流，同样触发）。
   - 复现：PC 自身 loopback（client→server）UDP `-P2/-P4` 即失败：`iperf3: error - unable to read from stream socket: Resource temporarily unavailable`。
   - 已验证 3.14 / 3.17.1 / 3.19 / 3.21 四个构建，全部失败；TCP `-P4` 正常。
   - 结论：这是 **PC 工具链缺陷，非板端问题**。
2. **替代方法**：板端作 UDP 多流 client 时，改用 **N 个并行单流进程**（N 个 PC server 端口 + 板端 N 个 `-P 1` 进程），双向则用 N×tx + N×rx。
   - 相关用例标记为 `-mproc`，note 中注明。
3. 板端 busybox **无 `pkill/pgrep/timeout`**（有 `killall/pidof`）；板端 client 一律**后台运行 + 轮询 + 超时强杀**，避免前台卡死阻塞串口。

## 4. 结果矩阵

### 4.1 Linux / STA

链路：PC 热点 `192.168.137.1` ↔ 板 `192.168.137.68`。单位 Mbps。

| 协议 | 角色 | 方向 | 单流 P1 | 多流 P4 |
|---|---|---|---|---|
| TCP | board-s | tx (板→PC) | 58.2 | 58.7 |
| TCP | board-s | rx (PC→板) | 54.6 | 53.9 |
| TCP | board-s | bidir | 板→PC 53.6 / PC→板 5.8 | 板→PC 43.8 / PC→板 10.7 |
| TCP | board-c | tx (板→PC) | 58.5 | 57.0 |
| TCP | board-c | rx (PC→板) | 46.4 | 45.2 |
| TCP | board-c | bidir | 板→PC 52.5 / PC→板 4.3 | 板→PC 41.6 / PC→板 8.6 |
| UDP | board-s | tx (板→PC) | 73.8 (丢包 4.6%) | 73.0 (0%) |
| UDP | board-s | rx (PC→板) | 57.9 (0%) | 56.6 (~0%) |
| UDP | board-s | bidir | 板→PC 69.1 (丢 5.9%) / PC→板 2.5 (丢 1.7%) | 板→PC 69.5 (~0%) / PC→板 2.2 (丢 2.5%) |
| UDP | board-c | tx (板→PC) | 73.8 (丢 1.5%) | 76.5 *(mproc)* |
| UDP | board-c | rx (PC→板) | 51.3 (~0%) | 54.1 *(mproc)* |
| UDP | board-c | bidir | 板→PC 68.7 / PC→板 2.5 *(mproc)* | 板→PC 71.1 / PC→板 9.5 *(mproc)* |

说明：`*(mproc)*` = 因 Cygwin server UDP `-P` 缺陷，改用并行单流进程测得的总和（见 §3）。

### 4.2 Linux / AP

链路：板端 AP `10.187.134.1/24`（hostapd + udhcpd，SSID `licheerv`）↔ PC USB 网卡 `10.187.134.2/24`。
AP 配置：`hw_mode=g`，channel 1，`ieee80211n=1`；**默认配置缺 `rsn_pairwise` 导致回落到 TKIP、HT 被禁用（802.11g）**，已通过 `/boot/hostapd.conf` 覆盖启用 `wpa_key_mgmt=WPA-PSK` + `rsn_pairwise=CCMP`，实测协商 **802.11n 65 Mbps**。
注意：使用 `/boot/hostapd.conf` 覆盖时，S30wifi 会跳过 IP 前缀推导，需同时提供 `/boot/wifi.ipv4_prefix`（本测试用 `10.187.134`）。

| 协议 | 角色 | 方向 | 单流 P1 | 多流 P4 |
|---|---|---|---|---|
| TCP | board-s | tx (板→PC) | 40.8 | 42.4 |
| TCP | board-s | rx (PC→板) | 32.8 | 32.1 |
| TCP | board-s | bidir | 板→PC 25.3 / PC→板 14.0 | 板→PC 24.3 / PC→板 15.1 |
| TCP | board-c | tx (板→PC) | 44.8 | 45.2 |
| TCP | board-c | rx (PC→板) | 40.9 | 40.5 |
| TCP | board-c | bidir | 板→PC 31.3 / PC→板 12.9 | 板→PC 27.3 / PC→板 15.4 |
| UDP | board-s | tx (板→PC) | 45.8 (丢 2.4%) | 47.4 (0%) |
| UDP | board-s | rx (PC→板) | 44.1 (~0%) | 44.4 (~0%) |
| UDP | board-s | bidir | 板→PC 35.3 (丢 2.3%) / PC→板 12.5 | 板→PC 36.9 / PC→板 11.2 |
| UDP | board-c | tx (板→PC) | 46.1 (丢 1.9%) | 44.8 *(mproc)* |
| UDP | board-c | rx (PC→板) | 42.9 (~0%) | 43.1 *(mproc)* |
| UDP | board-c | bidir | 板→PC 33.9 / PC→板 12.1 *(mproc)* | 板→PC 39.5 / PC→板 9.4 *(mproc)* |

PC 侧 AP 接口使用**静态 IP**（板端 udhcpd 未能成功下发，`ipconfig /renew` 超时），故设 `10.187.134.2/24`。

### 4.3 StarryOS / STA

板端 iperf3 **3.19.1**（内核日志噪声大，`starry_kernel::*`）；STA 连 PC 热点，DHCP 动态 IP（本轮为 `192.168.137.76`）。
StarryOS 较脆弱：**UDP 与多流 `-P4` 会导致板端卡死并把 WiFi 数据面搞坏**（需重启）。故 P4/UDP 大量跳过，仅 P1 可靠。

| 协议 | 角色 | 方向 | 单流 P1 | 多流 P4 |
|---|---|---|---|---|
| TCP | board-s | tx (板→PC) | 7.30 | 6.74 ⚠️退化（仅 1 流有数据） |
| TCP | board-s | rx (PC→板) | 20.3 | 21.3 |
| TCP | board-s | bidir | 板→PC 3.32 / PC→板 1.85 | 板→PC 6.36 / PC→板 6.36 |
| TCP | board-c | tx (板→PC) | 6.61 | ⏭️ 跳过（卡死板子） |
| TCP | board-c | rx (PC→板) | 16.0 | ⏭️ 跳过 |
| TCP | board-c | bidir | ❌ 中途停滞（无汇总） | ⏭️ 跳过 |
| UDP | board-s | tx/rx/bidir | ⏭️ 全部跳过：板端 server `unable to start stream listener` | ⏭️ |
| UDP | board-c | tx/rx/bidir | ⏭️ 全部跳过：板端 client UDP 卡死板子 | ⏭️ |

**结论**：StarryOS 仅 TCP 单流可用，吞吐显著低于 Linux（板端发送约 6–7 Mbps vs Linux 58 Mbps；接收约 16–20 Mbps vs Linux 46–55 Mbps）；多流与 UDP 在 StarryOS 上基本不可用。

### 4.4 StarryOS / AP

**未测试 / 不可用**。StarryOS 的 WiFi 模式在**镜像构建期固定**，无法像 Linux 那样通过 `/boot/wifi.*` 运行时切换；本镜像只准备了 STA。且该镜像/系统本身较脆弱（见 §4.3），AP 模式不具备可测条件，故 AP 全部用例记为 **SKIP（StarryOS 镜像未提供 AP / 运行时无法切换）**。

| 协议 | 角色 | 方向 | P1 | P4 |
|---|---|---|---|---|
| TCP/UDP | board-s / board-c | tx/rx/bidir | ⏭️ 全部跳过（StarryOS AP 不可用） | ⏭️ |

## 5. 跳过 / 缺失用例记录

### 5.1 因 PC 端工具缺陷改用替代方法（Linux，数据已取得）

| 用例 | 原因 | 处理 |
|---|---|---|
| Linux STA/AP board-c UDP `-P>1`（原生 `-P4`） | Cygwin iperf3 server 不支持 UDP `-P>1` | 用 `-mproc` 并行单流进程替代，数据已取得 |
| Linux STA/AP board-c UDP `bidir`（原生 `--bidir`） | 同上（UDP bidir=2 流） | 用 `-mproc`（N tx + N rx）替代，数据已取得 |

### 5.2 StarryOS 跳过用例（无数据）

| 用例 | 原因 | 处理 |
|---|---|---|
| StarryOS STA board-s UDP 全部（tx/rx/bidir × P1/P4，6 个） | 板端作 server 时 UDP 无法建立：`iperf3: error - unable to start stream listener` | 全部 SKIP |
| StarryOS STA board-c TCP `-P4`（tx/rx/bidir，3 个） | 板端作 client 时 `-P4` **卡死板子**并搞坏 WiFi（需重启） | 全部 SKIP |
| StarryOS STA board-c TCP `bidir-P1` | 跑至中途两个方向均掉到 0 并停滞，无最终汇总 | 记为 FAIL/退化 |
| StarryOS STA board-c UDP 全部（tx/rx/bidir × P1/P4，6 个） | 板端作 client 时 UDP **卡死板子**并搞坏 WiFi | 全部 SKIP |
| StarryOS AP 全部 | StarryOS WiFi 模式在镜像构建期固定，本镜像未提供 AP，运行时无法切换 | 全部 SKIP |

### 5.3 其它限制

| 项 | 说明 |
|---|---|
| Windows 端 iperf3 为 Cygwin 构建 | 所有结果的一端是 Cygwin iperf3；UDP `-P>1` server 缺陷见 §3 |
| StarryOS board-s UDP P1 实测（非跳过） | 已尝试一次，确认报错 `unable to start stream listener` 后跳过其余 |

## 6. 过程事件记录

- 2026-09-16：首次跑 Linux STA board-s TCP 组时，板子因**充电宝供电不足**（又插了其他设备）发生**掉电重启**，导致 `rx-P1` 中途失败、后续用例全部 connection timed out。已更换稳定电源后重跑，数据以重跑为准。
- 板端 UDP `-P4 -b 0` 曾**卡死前台进程并阻塞串口**，Linux 上已改为后台运行 + 轮询 + 超时强杀机制。
- StarryOS 测试期间：`board-c TCP -P4` 与 `board-c UDP` 均会**卡死板子**，并进一步导致 **WiFi 数据面失效**（内核陷入 `wlan0: requesting ARP for 192.168.137.1` 死循环、热点客户端掉线），需**断电重启**才能恢复。因此 StarryOS 的多流/UDP 用例按约定跳过。
- 供电建议：板子务必使用**独立稳定 5V 电源**，避免与其它设备共用充电宝（曾导致掉电重启、并有损坏 SD 卡风险）。

## 7. 结论

1. **Linux（Buildroot）网络性能正常**：STA 下单流 TCP 板端发送 ~58 Mbps、接收 ~55 Mbps；UDP 饱和可达 ~74 Mbps；AP 模式下（启用 CCMP/802.11n 65 Mbps 链路）单流 TCP ~41–45 Mbps。
2. **StarryOS 网络性能明显更弱且不稳定**：仅 TCP 单流可用，板端发送 ~6–7 Mbps、接收 ~16–20 Mbps；**多流（-P4）与 UDP 基本不可用**（会卡死系统并破坏 WiFi）。
3. **Cygwin iperf3 server 不支持 UDP `-P>1`**（Windows 端工具缺陷），board-c UDP 多流改用并行单流进程测得。
4. 所有数据的一端均为 **Windows 端 iperf3**，绝对值受其限制；**相对比较（Linux vs StarryOS）在同一工具条件下进行，具有可比性**。

## 8. 板端现场状态与遗留事项

- Linux SD 卡的 `/boot` 被修改过：新增 `wifi.ap`、`wifi.ipv4_prefix`（值 `10.187.134`）、`hostapd.conf`（启用 CCMP），并删除了 `wifi.sta`。**若需恢复 Linux STA**：删除 `wifi.ap`/`wifi.ipv4_prefix`/`hostapd.conf`，`touch /boot/wifi.sta`，写入 `wifi.ssid`/`wifi.pass`，再 `/etc/init.d/S30wifi stop; start`。
- PC 侧 `WLAN 2`（8188GU）残留静态 IP `10.187.134.2/24`（AP 测试遗留），可移除。
- 测试用 PC 端 iperf3：`tools\iperf3-win\iperf3.17.1_64\iperf3.exe`（另存有 3.14/3.19/3.21 供对照）。
