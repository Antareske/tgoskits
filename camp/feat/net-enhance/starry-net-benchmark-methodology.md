# Starry 网络优化项目：性能度量方案与 Linux 基线测试

> 目标：当 Starry 网络优化作为**独立项目**落地时，给出一套覆盖
> **吞吐 / 延迟 / PPS / CPU 效率 / 多核扩展 / 稳定性**的度量方法，使"优化前后"差异
> **可量化、可复现、可解释**，并且**同一套指标能在 Linux 上跑出基线**用于对照。
>
> 本文档基于最新 dev 分支状态（commit 6a857920）更新，反映已完成的 poll 
> 模型重构和待推进的性能优化工作。
>
> 配套分析见同目录 `starry-net-performance-analysis.md`（瓶颈定位）。本篇专讲**怎么测**。

---

## 0. 先确认一个前提：测试拓扑决定数据有没有意义

仓库当前 QEMU 默认用 **`-netdev user`（SLIRP 用户态网络）**
（`scripts/self-compile.sh:279`、`scripts/axbuild/src/rootfs/qemu.rs:23-28`）。

**SLIRP 不能用于性能测试**：它在 host 用户态做 NAT，吞吐被它自己限死（常见几百 Mbps 封顶），
且引入不可控的额外拷贝与调度抖动。任何在 SLIRP 上测出的“吞吐提升”都不可信。

性能测试必须换成下面三类拓扑之一（按推荐度排序）：

| 拓扑 | 说明 | 适合测什么 | 噪声 |
|------|------|-----------|------|
| **物理机 + 真实网卡**（e1000/ixgbe/realtek 直通或裸机） | 最接近 Linux 基线对比 | 终极吞吐/PPS/扩展性 | 最低 |
| **QEMU + TAP + vhost-net** | `-netdev tap,vhost=on` + `virtio-net-pci` | virtio 路径吞吐、多队列 | 中 |
| **QEMU + TAP（无 vhost）** | `scripts/net/qemu-tap-ifup.sh` 思路（见 bwbench README） | 功能正确性、相对趋势 | 较高 |

> 关键纪律：**Starry 与 Linux 基线必须跑在同一拓扑、同一网卡型号、同一 MTU、同一 CPU 绑定下**，
> 否则数字不可比。所有结论都要标注拓扑。

---

## 1. 指标体系（“能说明问题”的核心）

只测“一个 iperf 吞吐数字”说明不了问题。建议固定下面 6 个维度，每次优化都全测：

| 维度 | 指标 | 为什么重要 | 工具 |
|------|------|-----------|------|
| **吞吐** | TCP/UDP 单流 & 多流 Gbps | 大包搬运能力 | iperf3 / netperf TCP_STREAM |
| **PPS / 小包** | 64–256B UDP 包每秒 | 暴露每包开销（拷贝/加锁），小包最吃协议栈 | netperf UDP_RR、自研 pktgen 风格 |
| **延迟** | RR 往返 P50/P99/P999、`netperf TCP_RR` | 暴露调度/poll/yield hack 的尾延迟 | netperf *_RR、sockperf |
| **连接速率** | 短连接 QPS（accept/connect/close 速率） | 暴露 listen_table / 端口分配锁竞争 | nginx+wrk、ab、自研 connect loop |
| **CPU 效率** | **每 Gbit 消耗的 CPU cycles**、cycles/packet | 吞吐相同也要看是否更省 CPU；这是“追赶 Linux”最本质指标 | perf stat / 周期计数器 |
| **多核扩展** | 吞吐 vs 核数曲线、扩展比 | 验证去全局锁/多队列的成效（见分析文档 §3） | 上述工具 × N 并发 × taskset |

> **最能说明问题的两个派生指标：**
> 1. **cycles/byte 或 cycles/packet**——同样 1Gbps，Starry 花的 CPU 是 Linux 的几倍？这直接量化
>    “多拷贝 + 软件校验和 + 每包加锁”的代价。
> 2. **多核扩展比** = 吞吐(N核)/吞吐(1核)。Linux 接近线性，Starry 当前因全局大锁预计接近 1（不扩展）。
>    这张曲线最能体现 §3 锁拆分 / 多队列改造的价值。

---

## 2. 推荐的测试程序（应用层负载生成）

优先选**Starry 已经能跑 Linux ELF**这一事实，直接复用标准工具，保证和 Linux 基线用的是
**同一个二进制 / 同一套协议行为**：

### 2.1 主力工具
- **iperf3**：TCP/UDP 吞吐主力。`-P` 多流、`-R` 反向、`-u -b` UDP 限速扫 PPS、`--bidir` 双向。
  Starry 侧与 Linux 侧用同版本 iperf3，对端固定一台高性能 Linux。
- **netperf**：补 iperf3 的短板——`TCP_RR`/`UDP_RR`（延迟与事务率）、`TCP_CRR`（含连接建立的
  短连接速率，直接压 accept/connect 路径）。延迟分布比 iperf 更专业。
- **nginx + wrk / wrk2**：真实 HTTP 短连接 QPS 与延迟分布。仓库已有 nginx 相关冒烟资产
  （见 `www/nginx-*.md`），可作为“真实应用”场景，比纯 iperf 更有说服力。
- **sockperf**：专测延迟（under-load latency、ping-pong、throughput-latency 曲线），P99/P999 强。
- **redis-benchmark / memtier**（可选）：典型小包请求-响应，贴近实际服务负载。

### 2.2 小包 / PPS 专项
- **netperf UDP_RR + 多并发** 或 **iperf3 UDP 小包 + `-l 64`**：测每包开销。
- 若要打满线速找上限，host 侧用 **DPDK pktgen / TRex / MoonGen** 作为打流器对打 Starry
  接收（接收侧 PPS 是协议栈每包成本最纯粹的体现）。

### 2.3 仓库内已有、可复用的资产
- **`os/arceos/tools/bwbench_client`**：raw-frame 收发带宽基准（`bwbench_client [sender|receiver] [iface]`），
  绕过 TCP/UDP，直接量化**网卡 + 设备层**的裸帧吞吐——适合隔离驱动层 vs 协议栈层的瓶颈。
  README 已说明 guest 侧需配一个对应的 raw-frame app/test-suit case。
- **`apps/qperf`**：基于 `tgoskit-harness_kit` 的 QEMU plugin + analyzer，可产出
  **火焰图（flamegraph）**。用于定位 CPU 花在哪——配合 §1 的 cycles/packet 解释“为什么慢”。
- **`test-suit/starryos/qemu-smp1` 与 `qemu-smp4`**：现成的单核 / 4 核运行框架，
  天然适合做**多核扩展曲线**（同一负载分别在 smp1 / smp4 上跑）。

---

## 3. 用 eBPF / 内核观测手段“解释”差异

应用层工具给出**结果**（快了多少），eBPF 和内核内计数给出**原因**（为什么）。两者要配套。

### 3.1 Linux 基线侧（eBPF 在这里最强）
在 Linux 基线机上，用 eBPF 拆解协议栈成本，作为“理想路径”参照：

- **`bcc/bpftrace` tracepoint**：
  - `net:netif_receive_skb` / `net:net_dev_xmit`：收发包速率与大小分布。
  - `tcp:tcp_probe`、`sock:inet_sock_set_state`：TCP 状态机与窗口变化。
  - `skb:kfree_skb`（带 drop reason）：**丢包归因**——和 Starry 的丢包路径（Router 环满、ARP
    pending 满，见分析文档 §4）一一对照。
- **`bpftool` / `funclatency`（bcc）**：`tcp_sendmsg`/`tcp_recvmsg`/`__netif_receive_skb_core`
  的函数耗时分布，量化每包在协议栈各阶段的 ns。
- **`tcplife` / `tcpretrans` / `tcptop`（bcc）**：连接生命周期、重传、Top 流量，定位异常。
- **`perf stat -e cycles,instructions,cache-misses,LLC-load-misses`**：算 cycles/byte、IPC、
  cache 命中——直接量化“多拷贝”带来的 cache 与 cycle 浪费。
- **`perf record`+火焰图**：Linux 侧协议栈火焰图，与 Starry 的 qperf 火焰图并排对比，
  一眼看出 Starry 多出来的拷贝/锁/校验和热点。

> 注意：**eBPF 跑在 Linux 上**（无论是基线机，还是作为对打流量的对端/分析机）。
> Starry 自身没有 eBPF runtime，不要指望在 Starry 内部插 BPF。

### 3.2 Starry 侧的等价观测（自建“穷人版 eBPF”）
Starry 内没有 BPF，要用**埋点 + 计数器**达到同样的“可归因”效果（与分析文档 §4 的统计建议一致）：

- **per-socket / per-device 原子计数**：rx/tx packets+bytes、drop（按原因：环满 / ARP pending 满 /
  无路由 / 畸形包）、retrans、queue-full 次数、poll 次数。这相当于 Starry 版
  `kfree_skb drop reason`。
- **拷贝次数 / 路径计数**：在 §2.1 拷贝点（`to_vec`、各 `copy_from_slice`）加可开关的计数，
  直接量化“每包几次拷贝”。
- **CPU 周期采样**：用架构周期计数器在 `poll_interfaces`、`Interface.poll`、设备 recv/send
  前后取差值，输出各阶段 cycles——对应 Linux 的 `funclatency`。
- **锁竞争观测**：仓库已有 **`components/lockdep`**，可用于观测 `SERVICE`/`SOCKET_SET` 全局锁的
  持有/等待，量化多核下的锁竞争（验证 §3 锁拆分前后的差异）。
- **qperf 火焰图**：把上述热点可视化，作为 Starry 侧主分析产物。

---

## 4. “优化前后”如何呈现（报告结构）

每个优化点（或每个里程碑）产出一份固定结构的对比，保证可复现、可解释：

### 4.1 实验矩阵（每次全跑）
```
拓扑     ∈ {物理网卡, QEMU+vhost-tap}
对端     固定一台 Linux（记录型号/内核版本/网卡）
被测     ∈ {Linux 基线, Starry 优化前(baseline tag), Starry 优化后(this change)}
核数     ∈ {1, 2, 4, (8)}     ← 多核扩展曲线
负载     ∈ {TCP 单流, TCP 16流, UDP 64B PPS, TCP_RR 延迟, TCP_CRR 短连接, nginx+wrk}
MTU      固定（先 1500，再单独测 jumbo 若实现）
```

### 4.2 每格记录的数字
- 吞吐 Gbps（均值 + 标准差，≥5 次取统计）
- PPS（小包）
- 延迟 P50 / P99 / P999
- **CPU：cycles/byte、cycles/packet、整体 CPU 占用%**
- 丢包数 / 重传数（按原因）
- 多核扩展比

### 4.3 呈现方式
- **三条对比柱/线**：Linux 基线、Starry 优化前、Starry 优化后，并标注“达到 Linux 的百分比”。
- **多核扩展曲线**：吞吐 vs 核数，三方同图。
- **火焰图并排**：Linux perf vs Starry qperf，圈出被优化掉的热点。
- **归因表**：每项指标的变化对应到哪个改动（去拷贝 / 锁拆分 / offload / 批处理…）。

---

## 5. 落地清单（最小可用 → 完整）

**第一步（搭基线，1–2 周）**
1. 切换测试拓扑到 TAP+vhost 或物理网卡，废弃 SLIRP 测试结论。
2. 准备一台固定 Linux 对端 + 基线机，记录硬件/内核/网卡/MTU。
3. 在 Linux 上跑通 iperf3 / netperf / nginx+wrk + `perf stat` + 一套 bcc 工具，确立**基线数字**。
4. 在 Starry 上跑通同一组应用层工具（同二进制），先得到优化前 baseline tag 的数字。

**第二步（建可观测，与优化并行）**
5. Starry 侧加 per-socket/device 计数 + 拷贝计数 + 阶段 cycles 采样（对应分析文档阶段一）。
6. 接 qperf 火焰图；用 lockdep 观测全局锁竞争。
7. 复用 `bwbench_client` 隔离驱动层 vs 协议栈层。

**第三步（每次优化的固定动作）**
8. 按 §4 实验矩阵全跑，产出三方对比 + 多核曲线 + 火焰图 + 归因表。
9. 把多核扩展比和 cycles/packet 作为**项目级 KPI 主指标**逐里程碑跟踪。

---

## 6. 工具与指标对应速查

| 想说明的问题 | 工具 | 关键指标 |
|--------------|------|----------|
| 大包吞吐够不够 | iperf3 TCP `-P` | Gbps |
| 小包/每包开销 | netperf UDP_RR、pktgen/TRex | PPS、cycles/packet |
| 延迟与尾延迟 | netperf *_RR、sockperf | P50/P99/P999 |
| 短连接/accept 锁 | netperf TCP_CRR、wrk | conn/s |
| CPU 是否更省 | perf stat（Linux）/ 周期采样（Starry） | cycles/byte、IPC |
| 多核扩展 | 全套 × smp1/smp4 × taskset | 扩展比曲线 |
| 慢在哪 | perf+火焰图（Linux）/ qperf（Starry） | 热点函数 |
| 为什么丢包 | bcc kfree_skb（Linux）/ drop 计数（Starry） | 按原因丢包数 |
| 锁竞争多严重 | lockdep（Starry）/ bcc（Linux） | 锁等待时间 |
| 驱动层 vs 协议栈 | bwbench_client | 裸帧吞吐 |
