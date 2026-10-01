# StarryOS 网络栈 eBPF 性能监测调研

面向 SG2002（LicheeRV Nano）WiFi 上的 StarryOS 网络栈，调查三件事：专业 eBPF 网络监测
实际在测什么、怎么测；StarryOS 今天能支撑到什么程度；在此基础上可以做出什么（含
StarryOS 特有的做法）。

范围：ax-net 网络栈（协议栈边界、队列运行时、到驱动的交接）。aic8800 驱动内部暂不展开，
但 L0（SDIO/CMD53）作为栈与驱动之间的交接面纳入。

配套文档：`netmon-plan.md`（现有方案与阶段）、`netmon-tracker.md`（改动-测试-现象跟踪）。

---

## 1. 专业 eBPF 网络监测：测什么、怎么测

### 1.1 五个机制（比"挂哪个函数"更重要）

专业工具之间的差异主要不在挂点选择，而在这五个机制上。

**（1）让数据包自带上下文，而不是维护 per-packet 状态。**

`netstacklat`（Linux `bpf-examples`）不配对入口/出口：内核在收包最早点给 `skb->tstamp`
打上时间戳，此后每个钩子只算 `now - skb->tstamp`。零 per-packet 状态、零 map 查找、
零配对竞态。`pping` 同理——解析 TCP timestamp 选项（`TSval`/`TSecr`）在包内容里做匹配
算 RTT，同样不依赖内核状态。

这条是全局性的：**上下文载体决定了一个工具能不能做到无状态、无锁、可并发**。
反过来，凡是用"入口存时间戳、出口取时间戳"配对的实现，都会遇到单槽被并发覆写、
需要 per-thread key、需要陈旧窗口过滤这些问题。

**（2）核内 O(1) 有界聚合 + per-CPU。**

每次事件只做一次 map 查找 + 一次自增；直方图用 log2 分桶（`netstacklat`：35 个桶覆盖
`(2^(b-1), 2^b]`，另有 1 个 sum 槽用于算均值，合计 36 槽）。per-CPU map 规避原子操作，
用户态周期性 dump。**快路径不输出事件**——这是开销可控的根本原因，也是它和
"每条事件推一个 ringbuf 记录"的分水岭。

**（3）关闭时接近零开销的静态插桩。**

Linux tracepoint 由 static key 门控：关闭态是一条 NOP，启用时把调用点打补丁成 jump。
fentry/fexit 靠 ftrace + BTF，既不改进程文本也不陷入。这是 Linux 能在热路径里放几百个
tracepoint 的前提。

**（4）钩子集合可编程，导出映射在用户态。**

同一份 BPF 源按 hook 参数化编译成 N 个程序，用户态选择启用子集；到 Prometheus 之类的
映射写在用户态配置里（`netstacklat` 的 `ebpf_exporter` yaml 即此）。

**（5）分清"能测"与"该测"。**

计数与吞吐用内核已有的累计计数器（procfs / ethtool）就够，不需要 eBPF；**分布、延迟、
以及延迟该归因到哪一层**才是 eBPF 的战场。把已有的累计计数器再用 eBPF 数一遍是净负债。

### 1.2 维度分类与业界手段

| 族 | 具体维度 | 业界挂点/手段（Linux） | 可迁移性 |
| --- | --- | --- | --- |
| 连接生命周期 | 建连/接受/关闭时延 | `sock:inet_sock_set_state`（4.16 起，从 `inet_sk_set_state` 发出，对 TCP/UDP 通用）；**accept 侧没有 tracepoint**，bcc `tcpaccept` 只能用 `inet_csk_accept` 的 kprobe，其源码自述"直到找到合适方案为止" | 低——需要完整 socket 层与 TCP 状态机 |
| 吞吐 | 字节/包速率、每流 goodput | `net:net_dev_queue` / `netif_receive_skb` / `netif_rx`（短模板 4 字段：`skbaddr` `len` `name` `net_cookie`）；tc-bpf；XDP。**GRO/LRO/TSO 打开时，同一"包"在不同层计数不可比** | 中——分层计数的形状可借，机制不必 |
| 延迟 | RTT、单向时延、每层服务时间 | RTT：`tcp:tcp_probe`（`tcp_rcv_established` 第一条语句，携带 `srtt`/`snd_cwnd`/`ssthresh`，**无需读 sock**）。单向时延：`SO_TIMESTAMPING` + 共享时钟（需要 PTP/PHC，实践上的拦路虎）。分层：见下 | **高** |
| 丢包与重传 | 重传、丢包原因 | `tcp:tcp_retransmit_skb`、`tcp:tcp_send_reset`（带 `enum sk_rst_reason`）、**`skb:kfree_skb` 带 `enum skb_drop_reason`**（当前 124 项，含 `CPU_BACKLOG`/`QDISC_DROP`/`FULL_RING`/`NOMEM`）。SACK/DSACK 只有计数器没有事件流 | 中——"带原因的丢弃"形状可借，`/proc/net/dev` 的 4 个 error/drop 列是其简化版 |
| 队列与缓冲 | 队列深度、丢包、socket 队列占用 | `qdisc:qdisc_{enqueue,dequeue,drop}`（注意：`qdisc_dequeue` **没有 `dev_name` 也没有 `bytes`**，只有 `ifindex` `packets` `txq_state`）；`sock:sock_exceed_buf_limit`（9 字段） | **高**——与队列运行时的形状同构 |
| 拥塞控制 | cwnd、ssthresh、pacing rate | `tcp:tcp_probe` 直接带 `snd_cwnd`/`ssthresh`；`pacing_rate`/`delivery_rate` 只在 `TCP_INFO` 里 | 低——无 TCP 状态机则无意义 |
| **栈内服务时间** | 单包在栈内各段花了多久 | `netstacklat`：`fentry`/`fexit` 分层，读内核已打好的 `skb->tstamp` | **最高**——本项目的核心 |
| 错误与健康 | 驱动/NIC 错误、DMA/队列停摆 | **非 eBPF**：`ethtool -S`、sysfs 分项错误（`rx_missed_errors` 硬件侧 vs `rx_dropped` 驱动侧）、`IFLA_OFFLOAD_XSTATS`。唯一的内核停摆检测器 `napi:dql_stall_detected` 是 tracepoint | 中 |
| 唤醒与调度 | softirq/NAPI 时间、IRQ→处理延迟 | `irq:softirq_entry/exit/raise`（单字段 `vec`，`NET_RX=3` `NET_TX=2`；`softirq_raise` 头注释明说它是为了测 raise→run 延迟而存在）、`napi:napi_poll`（`napi` `dev_name` `work` `budget`）、`irq:irq_handler_entry/exit`（exit **不带 `action`**） | **高**——与队列运行时的 IRQ→poll 直接对应 |
| 系统级 | 网络占用的 CPU、IRQ 延迟 | 部分非 eBPF：`/proc/stat`、`/proc/net/softnet_stat`（第 2 列 backlog 丢弃、第 3 列 `time_squeeze` 预算耗尽、第 13 列 CPU id） | 中 |

**这一节的措辞是 Linux 的，不要直接搬进契约。** 上表按 `skb`/`napi`/`qdisc`/`softirq`
组织，那是 Linux 的结构。正确的用法是**问题从 Linux 实践里取，事件的措辞从 StarryOS
自己的结构里取**（poll group、generation、budget、owner_cpu）——见 `netmon-plan.md`
的设计原则 2。

两个判据要立住：

- **均值不够，要看分布。** 历史实测里"上行 94.8% 的 SDIO 写落在 20–50 ms 桶"这个结论，
  用均值完全看不出来——它会显示成一个可疑但不指向任何东西的中间值。
- **别把已有的累计计数器再用 eBPF 数一遍。** 见 §1.3。

### 1.3 非 eBPF 已覆盖什么，真正需要 eBPF 的又是什么

这是决定范围的一节。诚实的结论是：**"网络性能监测"里的大部分维度，读计数器就够了；
eBPF 的独特贡献比宣传口径窄得多。**

**非 eBPF 已覆盖**：每接口/每队列字节包计数（`/proc/net/dev`、sysfs
`/sys/class/net/*/statistics`、netdev genetlink `NETDEV_CMD_QSTATS_GET`）；分项错误
（sysfs 把 `rx_missed_errors`/`rx_dropped`/`rx_crc_errors`/… 分开给，`/proc/net/dev`
把它们**合并**了）；丢弃原因（`drop_monitor`/`NET_DM`，或 ftrace 打 `skb:kfree_skb`）；
CPU 侧收包压力（`/proc/net/softnet_stat`）；队列/中断拓扑（`ethtool -g/-c/-l/-T`、
`/proc/interrupts`）；qdisc backlog/丢弃（`tc -s qdisc`，但 `backlog` 是 dump 时刻的
**瞬时快照不是计数器**）；per-socket TCP 状态（`ss -ti` / `TCP_INFO` / `inet_diag`）；
重传聚合（`TcpRetransSegs`、`TcpExt` 分因计数）；自有 socket 的逐包时间戳
（`SO_TIMESTAMPING`）。

**真正需要 eBPF 的**：

| 维度 | 为什么别的做不到 |
| --- | --- |
| **逐包延迟分布**（p50/p99/max，按流或按层） | 所有计数器都是单调且无时间戳的。两次读只能给出区间**平均**，永远得不到分布、得不到分位数。`tcpi_rtt` 是 RFC 6298 的平滑标量（1/8 增益），轮询它是在采样一个被滤波过的量，不是在采样包 |
| **栈内逐包服务时间** | 没有任何接口暴露它（按排除法验证：SNMP MIB 是计数器无时间值；`gnet_stats_queue` 没有延迟字段；`softnet_stat` 是计数；`ethtool` 的直方图是 NIC 标准直方图不是包延迟）。`SO_TIMESTAMPING` 只标定**边界**（`RX_SOFTWARE` 是"驱动把手包交给收包栈之后"），中间那段——softirq → 汇聚 → 协议输入 → socket 队列——是不透明的 |
| **持续的、可过滤的、核内聚合的丢弃原因** | `NET_DM` 和 ftrace 能看到原因，但 `NET_DM` 是拉模式 tracer 且有自带的 1000 包/pCPU 有界队列，ftrace 吐文本、无法在核内聚合成按原因/按设备的直方图。**这是最锋利的一个真实缺口** |
| **跨层关联**（T 时刻的缓冲超限与 T 时刻的丢包） | 计数器不带顺序，因果不可恢复 |

对本项目最重要的一句：**对 Stack 内服务时间与逐包分布这两条，Linux 上只有 eBPF 能回答；
而它们恰好是"栈内监测器"可以天然拥有的东西**——因为监测器本身就坐在 Linux 需要探针
才能看进去的那一层里面。

### 1.4 开销量级（公开数据，用于选路线）

| 机制 | 单次开销 | 备注 |
| --- | --- | --- |
| `tp_btf`（raw tracepoint via `BPF_TRACE_RAW_TP`） | ~15 ns | |
| `fentry`/`fexit` | ~24 ns | |
| 静态 tracepoint | ~30–50 ns | |
| XDP | ~20–50 ns/包 | |
| **kprobe** | **~137 ns** | |

这些数字来自不同的测量环境，**不是同台对比**——可靠的是**量级排序**，绝对值仅供参考。
但排序本身印证了前面那条判断：kprobe 比静态 tracepoint 贵一个量级出头，那个差距就是
"陷阱 + 寄存器全量捕获"与"一次直接跳转"的差距。

netstacklat 的整体数字：**~225 ns/探针、0.81% CPU、尾部膨胀 ≤6%**（对比它的前代
"每包打时间戳"类工具是 **>100%**）。这个 0.81% 不是 fentry 快带来的，而是三个设计选择
带来的：**每包只打一次时间戳而不是两端各调一次时钟、核内固定指数直方图聚合且不让
任何逐包记录穿过到用户态、挂点少而只在层边界。**

输出通道的量级：ringbuf **22.57 M 记录/s** vs perfbuf **1.61 M**（默认配置，约 10×）。
但真正的教训不是"选 ringbuf"，而是**根本不要往外推逐包记录**：逐包推时间戳记录约
1 µs/条，而核内直方图是它两个数量级以下的成本。

---

## 2. 专业工具的方法学要点

**直方图分桶。**

| 方案 | 用在哪 | 误差 |
| --- | --- | --- |
| log2 指数桶（核内） | `netstacklat`、bpftrace `hist()` | **±33.3%** |
| 更细指数桶（如 `2^(1/8)`） | — | ±4.33%，需 267 桶 |
| 线性桶 | bpftrace `lhist()` | 桶宽内精确 |
| 固定桶 | `ebpf_exporter` 的 `bucket_type: exp2\|linear\|fixed` | 视配置 |
| HdrHistogram | 把记录推到用户态的工具 | 精度最高，代价最高 |

`netstacklat` 的取桶是**上取整**、区间右闭：`bucket = log2l(v)`，若
`bucket > 0 && (1<<bucket) < v` 则 `bucket += 1`，再截断到 `max_bucket`；
即桶 `b` 覆盖 `(2^(b-1), 2^b]`，另有 1 个 sum 槽记在 `max_bucket+1` 用于算均值。

对本项目：log2 桶一次 `leading_zeros` 就能定位、无循环，适合解释执行环境；
已知窄范围的现象（如那 20–50 ms 的时间片）可以用固定桶细看。
注意 `quantize()` 是 DTrace 的内建，bpftrace 只有 `hist()`/`lhist()`，两者别混。

**采样有三种，常被混为一谈。**

1. **探针内事件采样**：per-CPU 计数 + 掩码（`count & SAMPLE_MASK == 0`），
   计数类仍全量。现有 netmon 的 `SAMPLE_MASK` 就是这个，分工是对的。
2. **perf 事件采样率**：内核按频率（如 99 Hz）触发程序。最便宜，但它采样的是
   **调用**不是**包**。
3. **不采样**：丢弃、重传这类"计数即正确性"的维度必须全量，采样会得到错误的总数。

**输出通道。** perf buffer vs ringbuf 见 §1.4 的数字。但对聚合型监测两者都不需要
——直接轮询 map 即可，这也是最省的。

**主动与被动。** 主动探针（往网络里注入测量包）需要协议配合；被动测量只做旁路观察，
不扰动被观测流量。本项目应一律被动。

**一个容易误读的陷阱。** GRO/LRO/TSO 打开时，一层的"包"不是另一层的"包"。
任何在两个挂点上都按包长计数的探针，比较的不是同一件事——这是误读吞吐/丢弃比
最常见的原因。本项目的 `ProtocolEthernetFrame` 是定长结构、没有分片重组，
所以暂时没有这个问题，但一旦引入 offload 就要重新审视。

---

## 3. StarryOS 现状

### 3.1 eBPF 内核基础设施

| 项 | 实现 | 状态 |
| --- | --- | --- |
| 执行引擎 | `kbpf-basic` 0.6 + `rbpf` 0.4 | 解释执行；JIT 仅 x86_64（且页是 RWX）；riscv64 解释 |
| verifier | 无 Linux 式 verifier | `rbpf` 的 `verifier::check` 自述"与 Linux 无关、不检查控制流、不做寄存器类型" |
| 内存边界 | `register_allowed_memory(0..u64::MAX)` | **关掉了 rbpf 自身的边界检查**，源码带 TODO/FIXME |
| 指令预算 | 无 | **BPF 程序里一个循环会在 IRQ 上下文永久自旋** |
| `bpf(2)` 命令 | MAP_CREATE/LOOKUP/UPDATE/DELETE/GET_NEXT_KEY、PROG_LOAD、RAW_TRACEPOINT_OPEN、MAP_LOOKUP_AND_DELETE、MAP_FREEZE | 其余一律 `-EINVAL`；无 ATTACH/DETACH、无 LINK_CREATE、无 BTF、无 pin |
| map 类型 | ARRAY、PERCPU_ARRAY、HASH、PERCPU_HASH、LRU_HASH、LRU_PERCPU_HASH、QUEUE、STACK、PERF_EVENT_ARRAY、RINGBUF | 无 PROG_ARRAY（无尾调用）、无 LPM_TRIE、无 map-in-map、无 SK_STORAGE；`MAP_FREEZE` 是空操作 |
| helper | 24 个（kbpf 21 个 + StarryOS 补 14/16 + 113 别名到 4） | 含 `bpf_probe_read`(4)、`bpf_ktime_get_ns`(5，唯一时钟源)、`bpf_perf_event_output`(25)、`bpf_get_current_pid_tgid`(14)、ringbuf 系列(130-134) |
| 缺的 helper | 8 `smp_processor_id`、35 `get_current_task`、7 `prandom_u32`、112 `probe_read_user`、93/94 `spin_lock`、栈回溯系列 | 按需可补 |
| `bpf_probe_read` 语义 | **裸 memcpy** | 不是 fault-safe 拷贝；当时以为能接 `perf/nofault.rs`，实际不行——它是 aarch64 专用的单字页表走读，最终改为基于异常表实现（见 `netmon-plan.md` 阶段零） |
| attach 后端 | kprobe/kretprobe（`perf_event_open` + `PERF_EVENT_IOC_SET_BPF`）、cooked tracepoint（`PERF_TYPE_TRACEPOINT`，`config` = tracepoint id）、raw tracepoint（`BPF_RAW_TRACEPOINT_OPEN`，按名字）、uprobe | 无 fentry/fexit、无 tc/XDP/skb、无 uretprobe |

### 3.2 ax-tracepoint：形状与成本模型

`components/ax-tracepoint` 是一套 Linux `TRACE_EVENT` 形状的静态 tracepoint 框架：

- **形状**：`define_event_trace!` 生成 payload 结构体、`#[inline(always)]` 的调用点
  `trace_<name>(...)`、注册/注销函数、`.tracepoint` 链接段元数据、以及 `format` 文本。
- **用户态接口**：`/sys/kernel/debug/tracing/events/<system>/<event>/{id,format,enable,filter}`
  加 `trace_pipe`，与 Linux 同形。aya 走的就是读 `id` → `PERF_TYPE_TRACEPOINT` 这条路。
- **id 分配**：启动时按 `(name, system)` 排序后顺序赋号，**不是编译期常量，不可硬编码**。
  cooked attach 从 tracefs 读 id 所以安全；raw attach 是按裸名字线性扫描，因此事件命名
  要自带前缀（如 `net_*`）避免跨子系统重名。
- **filter**：payload 由 `tp-lexer` 的 schema 描述，能编译过滤表达式，失败保留旧表达式。
- **关闭态开销**：`callbacks_enabled` 的 `AtomicBool`（`Acquire` load）+ 一个预期不跳转的
  分支。**没有 static key / jump label / 文本打补丁**——这是有意的设计决定（早先的
  `ktracepoint` 会改活内核文本，本 crate 就是为了去掉这一点而存在），设计文档把
  "关闭路径成本是否可接受"留给后续 benchmark 决定。

**已验证可用**：树内三个 app 分别挂 cooked tracepoint（`mytrace` → `syscalls:sys_enter_openat`）、
raw tracepoint（`rawtp` → `sys_clone`）、raw tracepoint（`sched_trace` → `sched_switch`），
均通过 QEMU 冒烟。**框架侧不需要任何新工作。**

### 3.3 三条会成为设计约束的性质

1. **分发时并不持有快照锁**（这一点容易误读，包括本文件早先的版本）。
   `KernelExtTracePoint::acquire_snapshot()`（`tracepoint/registry.rs:96-107`）取
   `IrqMutex`、给该 epoch 的读者计数加一、克隆 `Arc`，然后**在返回租约前就 `drop` 掉了锁**；
   `read()` 的注释即"Runs a tracepoint read or callback dispatch without retaining the raw
   snapshot gate"。所以 BPF 程序是在"epoch 固定的 `Arc` + 读者计数"下运行，**不是在锁内**。
   代价从"可能死锁"降级为两件小事：`acquire_snapshot` 里那段关中断的短暂临界区，
   以及在调用点所在上下文里跑解释器的耗时。
   真正的限制来自 crate 本身的约定：回调不得注册/注销回调、改 filter、或递归触发
   同一注册表上的事件——而 BPF 只做 map 操作，不碰这些。**因此热路径可以直接发事件，
   不需要 `sched.rs` 那套 per-CPU 环 + worker 重放**（那套是为"必须离开当前上下文"的场景准备的）。
2. **payload 字段类型受限**：只有整数与整数数组（`TraceField`），没有 `&str`、`Vec`、
   枚举。字符串要用定长 `[u8; N]`。eBPF 侧按固定字节偏移读取，因此 payload 布局
   就是两侧的 ABI 契约，且 `format` 文本可以充当这个契约的机器可读描述。
3. **调用点不会被优化掉，但周围的记账代码可以被内联走**——这是静态 tracepoint 相对
   kprobe 的一个实际优势（见 §5）。

### 3.4 eBPF 侧的可用工具集（写程序时的事实约束）

- 时间源只有 `bpf_ktime_get_ns`。
- 无 `bpf_get_smp_processor_id`，per-CPU 语义目前只能靠 per-CPU map 隐式获得。
- 无尾调用、无 map-in-map，程序间无法共享状态机（只能靠 map）。
- 无 ABI 层面的 `ctx.arg(N)` 保证：cooked tracepoint 的 ctx 是记录字节流，
  按 `offset_of!` 布局读；raw tracepoint 的 ctx 是 `&[u64]` 参数数组。
- 解释执行且无指令预算：**每条事件执行的 BPF 指令数就是设计目标本身**。

---

## 4. 已有可观测设施：不该重复监测的部分

| 维度 | 现有出口 | 字段 |
| --- | --- | --- |
| 每接口收发计数 | `/proc/net/dev` ← `ax_net::net_dev_stats()` | 真实：`rx_bytes` `rx_packets` `rx_errors` `rx_dropped` `tx_bytes` `tx_packets` `tx_errors` `tx_dropped`；另 8 列硬编码 0 |
| ARP/邻居表 | `/proc/net/arp` | 真实 |
| 接口身份/状态/MTU/队列长度 | rtnetlink `RTM_GETLINK`、`SIOCGIFTXQLEN` | 真实 |
| 地址/路由/DNS | rtnetlink `RTM_GETADDR`/`RTM_GETROUTE` | 真实 |
| per-socket TCP 状态 | `getsockopt(TCP_INFO)` | 含 `retransmits` `probes` `backoff` `rto_micros` `snd_cwnd` `snd_wnd` `rcv_wnd` `notsent_bytes` `pmtu` |
| 内核符号 | `/proc/kallsyms` | 真实 |

明确不存在的：`/sys/class/net`、`IFLA_STATS64`、`/proc/net/{route,tcp,udp,unix,packet,sockstat}`、
`/proc/net/snmp`（是**全零桩**，smoltcp 不暴露按协议累计计数器）、网络 tracepoint（一个都没有）。

**一个值得注意的缺口**：`NetQueueStats` 的 12 个字段（`irq` `schedule` `missed`
`poll_batches` `budget_exhaustion` `spurious` `probe_deferred` `rearm_race` `owner_cpu`
`last_irq_cpu` `last_poll_cpu` `irq_to_poll_remote_wake`）**每个事件都在算，但没有任何
出口**——唯一实例被私有静态 `QUEUE_RUNTIME` 持有，`NetworkQueueRuntime::stats()` 虽然
是 `pub` 却拿不到句柄。这是"已有数据拿不到"而非"没有数据"。

**完全空白、只能靠新手段获得的**：任何延迟/分布（整个 ax-net 与驱动层没有任何计时设施）、
队列占用与背压深度、SDIO/CMD53 事务、WiFi 控制面事务、设备层硬件计数器。

---

## 5. 非侵入性设计：四条路线对比

"侵入性"要分成四个互相独立的维度看，混在一起谈会得不出结论：

| 路线 | 源码侵入 | 编译产物影响 | 运行期开销 | 鲁棒性 |
| --- | --- | --- | --- | --- |
| **kprobe/kretprobe**（现状） | 小（`#[inline(never)]` 注解） | **永久放弃这些函数的内联机会** | **每次命中：`ebreak` 陷阱 → 指令解码搬迁 → 离线单步 → 再进 BPF** | 依赖符号存活；受内联、泛型单态化、mangled 名、RVC 入口 2 字节对齐影响；attach 时要改内核文本权限 |
| **静态 tracepoint**（ax-tracepoint） | 较大（每个边界一处定义 + 一个调用点，payload 要显式列出） | 无（调用点是普通代码，不需要 `#[inline(never)]`） | 每次事件：`Acquire` 原子读 + 分支；命中时再加回调分发 | 与符号无关；不改内核文本；无对齐约束；payload 类型安全 |
| **raw tracepoint** | 同静态 tracepoint | 同上 | 同上，但 ctx 是裸 `&[u64]`，无 `format`、无 filter | 按裸名字解析，跨系统重名会静默撞车 |
| **核内直方图**（不挂 BPF） | 中（每个点几行 Histogram 调用） | 小 | 最低（一次分布内自增） | 最稳，但改一次要重编译；不可编程 |

要看清楚的一点：**kprobe 的"源码侵入小"是用"编译产物受损 + 每次命中付陷阱和单步"换来的。**
实测那 19% 吞吐下降（3 个 kprobe，12 → 9.77 Mbps）来自后者，而 `#[inline(never)]` 的代价
在没有探针时也一直在付。

反过来，静态 tracepoint 的"源码侵入大"换来的是：不改内核文本、无符号依赖、无对齐约束，
以及调用点周围记账代码仍可被内联。对一条要长期存在的监测设施来说，这个交换更划算。
历史上一度因为"对源码改动过大"否决了 raw tracepoint 方案，但当时的对比对象是
**驱动内部的 trait 钩子**（`WifiTrace`/`SdioTrace` 那套），那确实侵入大；而在栈边界上
放类型化的 tracepoint 与它不是一个量级。

**关于"稳定 ABI"这件事要说准。** Linux 文档（`bpf_design_QA.rst`）对"tracepoint 是不是
稳定 ABI"和"kprobe 挂点是不是稳定 ABI"的回答**都是 NO**——真正稳定的只有 BPF 指令集、
helper 集合、参数约定和返回码。所以"优先 tracepoint、其次 fentry、最后 kprobe"是一条
**惯例而非契约**（bcc 自己在 `tcpaccept` 里写"直到找到合适方案为止"只能用 kprobe）。
真正的差别是**谁拥有这个接口**：tracepoint 的 `format` 文件是维护者刻意选定、对用户态
可见的接口；而 kprobe/fentry 的挂点是内部函数命名空间，不属于任何人。

对 StarryOS 而言这条推论更直接：栈和监测器是同一个构建的产物，不存在"别人的接口"，
所以选择标准退化成纯粹的**成本与鲁棒性**——这两项都指向静态 tracepoint。

---

## 6. 推荐做法与可做的创新

### 6.1 主线：静态 tracepoint 承载事件，BPF 只做聚合

在 ax-net 的层边界定义 `net_*` 事件，payload 是类型化结构体；BPF 程序挂 cooked tracepoint，
每个程序只做"一次 map 查找 + 一次分桶"。框架、链接脚本、tracefs、perf attach、rbpf VM
全部现成，不需要动框架。

### 6.2 创新点 1：in-band per-packet 上下文载体（StarryOS 版 `skb->tstamp`）

`skb->tstamp` 在 StarryOS 没有对应物，但**载体是现成的**，而且不止一处：

- **协议栈内**：smoltcp 的 `PacketMeta { id: u32 }` 每包一个 u32，随包穿过 smoltcp 的缓冲。
  ax-net 已经在用它把 RX traffic-class 从 router 一路带到 socket，**不建侧表**
  （`net/ax-net/src/rx_meta.rs`）。
- **TX 到驱动**：`TxSubmitOptions`（`drivers/interface/rdif-eth/src/lib.rs:893`），文档原话是
  "Per-packet options passed across the runtime transmit boundary"——一个 `Copy` 的小结构，
  本来就每包一次穿过协议→队列→驱动。**这就是 TX 侧载体的位置。**
- **RX 自驱动**：`RxCompletion { buffer, packet_len }`（同文件 `:994`），"One completed receive
  buffer returned by a hardware queue"，由驱动填写、经 `ProtocolRxFrame` 上浮。
  **这就是 RX 侧载体的位置**，而且由驱动打时间戳是语义上最正确的做法——离硬件最近。

要澄清一个容易搞错的地方：帧类型 `ProtocolEthernetFrame` 是"非 DMA 端口与测试用的
兼容内联帧"，真正的队列路径**不经过它**（`EthernetFramePort::transmit_frame_with_options`
直接把数据填进 DMA 存储，`receive_with` 原地消费）。所以载体不该加在那里。

代价要说清楚：`TxSubmitOptions` 与 `RxCompletion` 位于可移植驱动接口 crate，
加字段会波及所有驱动（aic8800/e1000/rtl8125/fxmac）。设计上应让它**可选且有默认值**
——驱动不填就报 0，监测退化为"没有跨层数据"，而不是给出错误数据。

于是每个事件都退化成**无状态观察者**：只发 `now - carrier_ts`，不需要入口/出口配对、
不需要 per-thread key、不受并发覆写影响。这直接根治当前设计里单槽时间戳的固有缺陷
（app 里的 `TS_PORT`/`TS_SDIO`、历史 `WR_ENTRY` 被 WPA2 握手并发覆写），也是历史教训里
"应当一开始就用 per-thread key"那条的真正解法——不需要 key，因为没有跨调用的状态。

载体单位的选择：u32 纳秒覆盖 4.29 s 后回绕，而目标现象的尺度是 300 µs–50 ms，
回绕可检测（`stamp > now` 即判定回绕）；或改用步长更大的单位把覆盖窗口拉到分钟级。

### 6.3 创新点 2：记账与事件合一

`NetQueueStats` 今天"算了但拿不到"。与其为了 eBPF 往快路径另外塞探针，不如让边界上
**一次调用同时更新 stat 并触发 tracepoint**：procfs 计数、eBPF 直方图、用户态可读性
共用一条通路，快路径只多一个已有形状的调用点。顺带把 `/sys/kernel/debug/net_queue` 这类导出
（队列侧证据：`irq_to_poll_remote_wake`、`budget_exhaustion`、`missed`、`rearm_race`）
变成顺手可得的东西。

### 6.4 创新点 3：测量在核内，聚合在 BPF

栈是我们自己的，可以让 payload 直接携带**最终量**（`latency_ns`）而不是把原始时间戳
丢给 BPF 去算。测量逻辑留在 Rust（可单元测试），BPF 退化成"一次查找 + 一次分桶"。

在 rbpf 解释执行、无 JIT、无 verifier 的前提下，这不是洁癖而是必需：**每条事件执行的
BPF 指令数直接就是开销**。顺带避开 `ctx.arg(N)` 的 ABI 与寄存器读取坑（历史在
RISC-V 的 byval 传参上踩过）。

### 6.5 创新点 4：把关闭态做成真正的零开销

ax-tracepoint 的设计文档把"关闭路径是否该用 static key / jump label"留作待定
（理由是改活内核文本有风险）。Linux 的做法正相反，且已经证明可行。**值得看清的是：
Linux 的 tracepoint 用的是两套独立机制，而不是一套。**

- **门控（`struct static_key_false key`）**：`static_branch_unlikely()` 编译成一条
  5 字节 NOP 走直线；启用时把这条 NOP 打补丁成跳转。**关闭态没有内存读、没有分支、
  没有调用。**
- **分发（`static_call_key`）**：启用后不是间接调用，而是静态调用；由
  `tracepoint_update_call()` 决定目标——**只挂了一个探针时直接指向那个探针函数**
  （BPF 程序也只是 `->funcs` 里的一个条目，所以单工具部署天然吃到这条快路），
  两个以上才退回 `__traceiter_##name` 循环。x86_64 上 `DEFINE_STATIC_CALL_RET0`
  甚至把 5 字节 `CALL` 改成 5 字节 `xor eax,eax`，完全消除调用开销。
- **退化路径**：没有 `CONFIG_JUMP_LABEL` 时，`static_branch_unlikely` 退化成
  `unlikely_notrace(static_key_enabled(&key))`——一次加载加测试加分支。
  **这正是 ax-tracepoint 今天的状态**：它相当于"没有 jump label 的 Linux tracepoint"。

对网络热路径（每包过一次的事件）而言，一个 `Acquire` 读加分支乘以每秒上万个包就不再
是零头。可行的中间形态：给每个调用点一个可写的分支目标而不是改指令文本，既避免文本
补丁的风险又把关闭态降到一次直接跳转；或只对声明为 hot 的事件启用补丁式实现。
这是框架级改动，**是否值得做应当先有一次关闭态开销的测量来定**——ax-tracepoint 的
设计文档也是这么留的口子。

### 6.6 创新点 5：schema 驱动的监测

payload 结构体 + `format` 文本已经构成一份机器可读的 schema。可以据此让 loader
**自动生成**分桶槽位、用户态解析、以及过滤条件，于是"加一个指标"退化成"加一个
结构体 + 一个调用点"，不需要同时改 eBPF 程序和 loader。这与 Linux 需要 CO-RE 来解决
版本漂移不同——在单次构建内，schema 就是编译器保证的。

---

## 7. 需要补的基础设施

| 项 | Linux 对应 | StarryOS 现状 | 为本项目要做什么 |
| --- | --- | --- | --- |
| per-CPU 标识 | `bpf_get_smp_processor_id` (8) | 无 | 补上；per-CPU 配对与正确性需要 |
| 故障安全的读 | `bpf_probe_read_kernel` 带 fault 处理 | helper 4 是裸 memcpy | 已实现（异常表 + 四架构拷贝循环）；否则一次坏指针就是内核崩溃 |
| 指令预算 / 有界循环 | verifier 的 bounded loop + 复杂度上限 | 无 | 在 rbpf 包装层加指令计数上限；**IRQ 上下文自旋是当前最现实的风险** |
| 尾调用 | `BPF_MAP_TYPE_PROG_ARRAY` + `bpf_tail_call` | 无 | 若走"每钩子一程序 + 共享子程序"再补 |
| 事件命名空间 | tracepoint 的 `system:name` | raw attach 只按裸名字线性扫描 | `net_*` 前缀约定（或给 `find_ext_tracepoint_by_name` 加子系统限定） |
| map 冻结 | `BPF_MAP_FREEZE` 后不可写 | 空操作 | 低优先级 |
| ringbuf 兼容性 | libbpf 布局 | 有实现但**无任何 app 用过，mmap 布局未验证** | 若要用 ringbuf 输出明细事件再验证 |
| 关闭态零开销 | static key / jump label | `AtomicBool` + 分支 | 见 §6.5，先测量再决定 |

其中**指令预算**与**故障安全的读**是无论走哪条路线都该补的——当前 eBPF 子系统在这两点上
的暴露面，比网络监测这一个用例更大。

---

## 8. SG2002 上板考量

- 目标配置：`os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml`；
  WiFi 端到端冒烟在 `test-suit/starryos/board-aka-00-sg2002/wifi-iperf-smoke/`；
  吞吐对比用 `apps/starry/network-throughput/`。
- riscv64 无 JIT → 解释执行，指令数是开销主项，采样与开关策略必须设计进去。
- 板级构建与 QEMU 构建的符号集合不同（SDIO/WiFi 符号只在板上存在），挂点复核要在
  板级内核上做。静态 tracepoint 与符号无关，这一点上比 kprobe 省事。
- **吞吐验收必须在监测关闭时采集**（历史 19% 的教训）；延迟分布可以在开启时采集，
  但要同时记录采样率与开关状态，否则数字不可比。
- `/proc/net/dev` 与 eBPF 的分工要写死：前者是吞吐的验收口径，后者是延迟与归属的
  诊断口径，两者不互相替代。

---

## 9. 风险与开放问题

| # | 问题 | 说明 |
| --- | --- | --- |
| R1 | BPF 在 tracepoint 读侧锁内执行 | 网络热路径多处在持锁或 IRQ 上下文，可能必须走延迟发射（per-CPU 环 + worker），这会影响"无状态观察者"能省多少 |
| R2 | 解释执行 + 无 verifier + 无指令预算 | 探针本身可能成为故障源；这是要先补基础设施再谈功能的原因 |
| R3 | payload 布局是两侧隐式 ABI | 加字段会移动偏移；`format` 文本可当契约，但需要约定如何校验 |
| R4 | `#[inline(never)]` 的既有注解 | 走 tracepoint 路线后可以撤掉，但要确认撤掉后监测仍按预期工作（这本身是一次对照实验） |
| R5 | 单核还是多核、per-CPU 语义 | 影响时间戳载体与 per-CPU map 的必要性，需在上板前确认 |
| R6 | 采样率与分布形状的关系 | 采样对分布形状的影响需要一次实测确认，不能假设无偏 |

## 10. 待办

1. 确认 SG2002 的核数与 per-CPU 语义（影响载体与 per-CPU map 的必要性）。
2. 量一次 tracepoint 关闭态开销（vs kprobe 关闭态），为 §6.5 提供判据。
3. 在 ax-net 上落一条最小端到端链路（一个 `net_*` 事件 + 一个 BPF 直方图 + loader），
   验证 §3.3 的锁约束与 §6.2 的载体在真实路径上的可行性。
4. 决定挂点清单：从 §1.2 里"可迁移性高"的三族（栈内服务时间、队列与缓冲、唤醒与调度）
   落候选，并按 §1.3 排除掉非 eBPF 已覆盖的维度。

## 11. 参考

**工具与方法**
- `netstacklat` — https://github.com/xdp-project/bpf-examples（`netstacklat/`；本仓库
  `www/bpf-examples/netstacklat/` 有副本）
- `pping` — https://github.com/xdp-project/bpf-examples（`pping/`；本仓库有副本）
- bcc / bpftrace 工具集（`tcpconnect` `tcplife` `tcpaccept` `tcpretrans` `tcpdrop`
  `gethostlatency` `ssllatency` 等）
- Linux BPF 设计问答（tracepoint/kprobe 均非稳定 ABI）— `Documentation/bpf/bpf_design_QA.rst`
- static keys 文档 — https://docs.kernel.org/staging/static-keys.html

**内核源码（挂点与字段的事实来源）**
- `include/trace/events/{net,skb,sock,tcp,qdisc,irq,napi}.h`
- `include/net/dropreason-core.h`（`enum skb_drop_reason`）
- `net/core/net-traces.c`（所有网络 tracepoint 汇聚的编译单元）
- `kernel/bpf/trampoline.c`（fentry/fexit 的 trampoline）
- `kernel/tracepoint.c` + `include/linux/tracepoint.h`（static key + static call）

**本项目**
- `components/ax-tracepoint/`（含 `docs/design/ax-tracepoint.md`）
- `os/StarryOS/kernel/src/tracepoint/`、`os/StarryOS/kernel/src/perf/{tracepoint,raw_tracepoint,kprobe,bpf}.rs`
- `net/ax-net/src/{rx_meta.rs,queue_runtime/,device/driver.rs,router.rs}`
- `apps/starry/ebpf/netmon/`（现有 kprobe 版监测器）
