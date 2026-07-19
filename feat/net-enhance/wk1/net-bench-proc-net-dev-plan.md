# net-bench 弃用 eBPF net_stats、转用 /proc/net/dev 的调整计划

## 背景

经过对 StarryOS 网络栈各层计数位置的技术调查和与 Linux 内核统计实现的对比分析，
得出以下结论：

1. StarryOS 在 `DeviceHandle`（`net/ax-net/src/router.rs`）中已有 per-interface
   的 `rx_bytes`/`tx_bytes`/`rx_packets`/`tx_packets` 四个 `AtomicU64` 计数器，
   并通过 `render_proc_net_dev()`（`os/StarryOS/kernel/src/pseudofs/proc.rs`）
   以标准 Linux `/proc/net/dev` 格式暴露给用户态。

2. 当前这些计数器在 L3（IP 帧）层级累加，而非 Linux 标准的 L2（Ethernet 帧）层级。
   每包差 14 字节（Ethernet 头 DMAC+SMAC+EtherType），需要修复以与 Linux 语义对齐。

3. eBPF net_stats 工具试图在 smoltcp phy 层（`TxToken`/`RxToken::consume`）
   通过 kprobe 实现相同的计数功能，但面临 RX bytes 因 `RxToken` 内联导致 struct
   布局偏移无法确定的阻塞问题。

4. net-bench 实际上以 iperf3 的吞吐量数据（JSON 输出）为主要指标来源；
   net_stats 是计划中的辅助数据源，但当前 summarize.py 中的解析代码与 phy 层
   重构后的输出格式不兼容（字段名从 per-protocol 变为 global aggregate）。

## 核心决策

**弃用 eBPF net_stats 作为 net-bench 的字节/包计数来源，改用已有的
DeviceHandle 内置计数器（通过 /proc/net/dev 读取）。**

理由：
- DeviceHandle 的计数器和 eBPF net_stats 做的是同一件事（统计收发包字节数），
  但 DeviceHandle 计数器在知道字节数的代码行直接 `fetch_add`，不存在任何 ABI、
  内联、struct 布局的不确定性。
- `/proc/net/dev` 已经是 Linux 生态的标准接口，net-bench 的 host 侧脚本和
  summarize.py 解析它比解析自定义 `NET_STATS_BEGIN/END` 格式更自然。
- 修复 L3→L2 语义对齐是一个改动同时修复两个数据源（内置计数器和 eBPF net_stats
  如果继续存在的话），但内置计数器更可靠。

## 需要改进的点

### 1. 修复 /proc/net/dev 的计数层级（L3 → L2）— 高优先级

**问题**：当前 `DeviceHandle::count_tx()` / `count_rx()` 在 Router 层被调用，
传入的是 IP 帧长度（Ethernet 头已被 `EthernetDevice::handle_frame()` 剥离）。
Linux 的 `/proc/net/dev` 中 rx_bytes/tx_bytes 按 IEEE 802.3 规范定义为
Ethernet 帧长度（不含 FCS）。

**修复位置**：
- `net/ax-net/src/device/ethernet.rs` 的 `handle_frame()` — RX 路径。
  此处 `rx_buf.packet_len()` 是完整的 Ethernet 帧长度（L2），
  `frame.payload().len()` 是 IP 负载长度（L3）。需在此处对 DeviceHandle
  计数器做 L2 累加。
- 同一文件的 `send_to()` — TX 路径。此处 `repr.buffer_len() + size` 是完整的
  Ethernet 帧长度（L2）。
- 同时移除 Router 层（`router.rs` 的 `device_rx_worker`、`enqueue_tx`、
  `dispatch` 中）对 `count_tx`/`count_rx` 的调用，或将调用点移至 EthernetDevice
  层并传入 L2 长度。

**Loopback**：loopback 不经过 EthernetDevice，没有 Ethernet 头。其 L2 长度
等于 L3 长度，语义正确（Linux 的 lo 接口也不计 Ethernet 头）。loopback 路径上
的 `count_tx`/`count_rx` 调用保留在 Router 层即可。

### 2. net-bench 集成改造 — 高优先级

**当前状态**：
- net-bench 的核心吞吐数据来自 iperf3（`net-bench-common.sh`），不依赖 net_stats。
- `summarize.py` 中有 `parse_netstats()` 和 `render_netstats()` 函数，
  试图解析 `NET_STATS_BEGIN/END` 块，但字段名与当前 phy 层重构后的输出不兼容。
- net-bench 的 QEMU 配置（`qemu/*.toml`）中并未启动 net_stats，
  net_stats 有自己独立的 QEMU 测试配置（`ebpf/net_stats/qemu-*.toml`）。

**改造方案**：
- 从 `summarize.py` 中移除 net_stats 解析代码（或保留但标记为 deprecated），
  改为在 guest 测试脚本中通过 `cat /proc/net/dev` 获取 per-interface 统计，
  输出为 summarize.py 可解析的标记格式。
- 在 `net-bench-common.sh` 的 `run_test()` 调用前后分别采集 `/proc/net/dev`
  快照，差值即为该次测试的收发字节/包数。
- 或者：依赖 iperf3 自身的字节计数（`sum_received`/`sum_sent` 中的 `bytes`
  字段），这已经是应用层确认的传输量。需要明确区分"iperf3 报告的传输量
  （应用层）"和"网卡实际收发的字节量（含协议头）"——两者语义不同，各有用途。

### 3. 清理 net_stats eBPF 工具（可选）— 中优先级

**可以考虑保留的场景**：
- 作为独立的功能验证工具（`--test` 模式验证探头能正常挂载和触发）——
  这对验证 StarryOS 的 eBPF/kprobe 基础设施本身有测试价值。
- 未来如果需要 per-protocol 分解（TCP vs UDP vs ICMP），在 phy 层做不到
  （phy 层只看到 IP 帧），需要另外设计。

**可以移除或简化的场景**：
- 不再作为 net-bench 吞吐测试的数据源。
- 当前的 `NET_STATS_BEGIN/END` 输出格式可以保留，但从 net-bench 的 summarize.py
  中移除解析依赖。
- `summarize.py` 中的 `NetStatsSnapshot`、`parse_netstats()`、`render_netstats()`
  函数可移除。

### 4. 内置计数器与 eBPF net_stats 的收敛 — 低优先级

如果未来仍希望 eBPF 工具提供网络统计的观测能力，可以考虑：
- eBPF 程序不再自己计数，而是读取 DeviceHandle 中已有的 `AtomicU64` 计数器
  （通过读取内核符号地址或通过 eBPF map 中转）。
- 或者将 eBPF net_stats 重新定位为"per-socket / per-connection 级别的流量
  观测"（类似 Cilium 的 per-flow CT 统计），而非"per-interface 级别的设备统计"。

## 对 eBPF net_stats 现有代码的处理

本次计划**暂不修改** `apps/starry/ebpf/net_stats/` 下的代码。先完成内置计数器
的 L2 修复和 net-bench 集成改造，验证 `/proc/net/dev` 作为吞吐记录来源可行后，
再决定 net_stats eBPF 工具的去留。

## 相关文件

### 需要修改
- `net/ax-net/src/device/ethernet.rs` — 在 `handle_frame()` 和 `send_to()` 中
  接入 L2 层计数
- `net/ax-net/src/router.rs` — 调整 DeviceHandle 计数器调用点，区分 L2/L3 路径
- `apps/starry/net-bench/core/net-bench-common.sh` — 添加 `/proc/net/dev` 快照采集
- `apps/starry/net-bench/core/summarize.py` — 移除 net_stats 解析，添加
  `/proc/net/dev` 差值计算

### 无需修改
- `apps/starry/ebpf/net_stats/` — 暂不修改
- `apps/starry/net-bench/run.sh` — 入口逻辑不变
- `apps/starry/net-bench/prebuild.sh` — 不依赖 net_stats 二进制
- `os/StarryOS/kernel/src/pseudofs/proc.rs` — `/proc/net/dev` 渲染格式不变

## 验证方式

1. 在 QEMU 中启动 StarryOS，运行 iperf3 测试，同时采集 `/proc/net/dev` 快照
2. 确认 `/proc/net/dev` 的 rx_bytes/tx_bytes 增量与 iperf3 报告的数据量在
   合理关系内（rx_bytes > iperf3 payload bytes，差值为协议头开销）
3. 对比 Linux 基线（`run-linux-baseline.sh`）的 `/proc/net/dev` 数据，
   验证同一测试场景下两系统的统计值可直接对比
