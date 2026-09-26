# netstacklat 研究：借鉴思路实现 sg2002 WiFi 驱动性能监测 eBPF 工具

**日期**：2026-07-25

**分支**：`sg2002/wifi-ebpf`（基于 dev `66c9a2f13`）

**参考**：
- `../sg2002-wifi.md`（WiFi 驱动优化报告，BattiestStone4）
- `../wt-feat-net-enhance/apps/starry/ebpf/net_stats/README.md`（StarryOS eBPF 约束）
- `./netstacklat/bpf-examples/netstacklat/`（cloned source）
- `wt-sg2002-wifi/www/sg2002-wifi-performance-analysis.md`（初步调查-性能分析）
- `wt-sg2002-wifi/www/wifi-optimization-analysis.md`（初步调查-优化分析）
- `wt-sg2002-wifi/www/wifi-analysis.md`（初步调查-驱动分析）

## 目录

1. [netstacklat 核心设计分析](#1-netstacklat-核心设计分析)
2. [StarryOS eBPF 基础设施与约束](#2-starryos-ebpf-基础设施与约束)
3. [WiFi 驱动可监测点分析](#3-wifi-驱动可监测点分析)
4. [方案设计：基于 tracepoint 的 WiFi 性能 eBPF 工具](#4-方案设计基于-tracepoint-的-wifi-性能-ebpf-工具)
5. [实现计划](#5-实现计划)

---

## 1. netstacklat 核心设计分析

### 1.1 架构概述

netstacklat（`xdp-project/bpf-examples/netstacklat/`）是一个 Linux 网络栈延迟监测工具，用于测量**接收数据包从网卡到应用穿越内核网络栈各阶段的耗时**。核心架构：

```
┌────────────┐  SOF_TIMESTAMPING_RX_SOFTWARE (系统级开关)
│ user sock   │  → kernel 为所有入站包打早期时间戳 skb->tstamp
└────────────┘
┌────────────────────────────────────────────────────────────────┐
│ eBPF fentry/fexit probes at various network stack points      │
│                                                                │
│  ip_rcv_core ─→ latency = now - skb->tstamp  (ip-start)      │
│  tcp_v4_rcv  ─→ latency = now - skb->tstamp  (tcp-start)     │
│  tcp_queue_rcv(fexit) ─→ ...               (tcp-sock-enqueued)│
│  tcp_recv_timestamp ─→ ...                  (tcp-sock-read)   │
│                                                                │
│  Each probes: compute latency → write exp2 histogram          │
└────────────────────────────────────────────────────────────────┘
```

### 1.2 关键设计思想

#### 1.2.1 零状态跟踪：时间戳载体复用

netstacklat 最精妙的设计：**不需要维护 per-packet 状态**。借助 Linux kernel 的 `SOF_TIMESTAMPING_RX_SOFTWARE` 机制，任何 socket 打开此选项后，每笔入站 packet 的 `skb->tstamp` 字段在栈早期就被打上时间戳。后续每个 probe point 只需计算 `bpf_ktime_get_tai_ns() - skb->tstamp` 即得延迟，无需 eBPF map 记录 entry 时间。

**对 StarryOS 的启示**：StarryOS 没有 `SOF_TIMESTAMPING_RX_SOFTWARE` 基础设施。但我们可以借鉴"时间戳载体"思想，在驱动层手动打时间戳——具体策略见第4节。

#### 1.2.2 指数直方图聚合

不输出原始 per-packet 延迟，而是写 base-2 指数直方图（34 个 bucket，覆盖 ns→~17s）。每个 bucket 跨度为上一 bucket 的两倍。这使得：

- 输出紧凑：无论多少数据包，输出最多 34 行/histogram
- 延迟分布清晰：log2 histogram 直接暴露长尾 (tail latency)
- ebpf_exporter 兼容的 `sum key` 模式（bucket 索引 MAX+1 存总和，用于计算平均值）

#### 1.2.3 分层诊断能力

通过不同 probe point 间延迟的**差值**定位瓶颈层级：

- `sock-read` 高但 `sock-enqueued` 低 → 应用层读取慢
- `sock-enqueued` 高但 `tcp-start` 低 → TCP 层处理慢 (OOO, 拥塞控制)
- `tcp-start` 高但 `ip-start` 低 → IP 层处理慢 (Netfilter, 路由)
- `ip-start` 高 → 网卡软中断/NAPI/驱动层慢

### 1.3 核心源码结构

| 文件 | 职责 |
|------|------|
| `netstacklat.bpf.c` | eBPF 内核侧程序：7 个 probe point，histogram map，filter 逻辑 |
| `netstacklat.h` | 共享结构体：`hist_key`, `netstacklat_bpf_config`, `netstacklat_hook` 枚举 |
| `netstacklat.c` | 用户态 loader：解析参数，加载 BPF 程序，定时读取+打印 histogram |
| `bits.bpf.h` | `log2`/`log2l` 实现（eBPF 无浮点指令） |
| `netstacklat.yaml` | ebpf_exporter 配置（Prometheus 导出） |

### 1.4 Map 和 Probe Point 细节

**核心 Map**：

```c
// PERCPU_HASH: key = {cgroup, ifindex, hook, bucket}, value = u64 count
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_HASH);
    __uint(max_entries, HIST_NBUCKETS * NETSTACKLAT_N_HOOKS * 64);
    __type(key, struct hist_key);
    __type(value, u64);
} netstack_latency_seconds;

struct hist_key {
    __u64 cgroup;   // 0 if not groupby
    __u32 ifindex;  // 0 if not groupby
    __u16 hook;     // enum netstacklat_hook
    __u16 bucket;   // histogram bucket index (MUST be last for ebpf_exporter)
};
```

**Probe Points**（全部使用 fentry/fexit，基于 BTF）：

| Hook | 挂载点 | 类型 | 语义 |
|------|--------|------|------|
| `ip-start` | `ip_rcv_core` + `ip6_rcv_core` | fentry | 到达 IP 栈 |
| `tcp-start` | `tcp_v4_rcv` + `tcp_v6_rcv` | fentry | 到达 TCP 层 |
| `udp-start` | `udp_rcv` + `udpv6_rcv` | fentry | 到达 UDP 层 |
| `tcp-socket-enqueued` | `tcp_queue_rcv` | fexit | 入队到 socket |
| `udp-socket-enqueued` | `__udp_enqueue_schedule_skb` | fexit | 入队到 socket |
| `tcp-socket-read` | `tcp_recv_timestamp` | fentry | 用户态读取 |
| `udp-socket-read` | `skb_consume_udp` | fentry | 用户态读取 |

---

## 2. StarryOS eBPF 基础设施与约束

### 2.1 现有 eBPF 框架

StarryOS 使用 **aya-rs**（Rust eBPF 框架），通过 `rbpf` 解释器执行（SG2002 RISC-V 无 BPF JIT）。完整的三层工作区结构：

```
apps/starry/ebpf/<app>/
├── <app>-common/         # 共享常量/结构体 (no_std, 被 eBPF 和 loader 共同依赖)
│   └── src/lib.rs
├── <app>-ebpf/           # eBPF 内核侧程序 (aya-ebpf, no_std)
│   ├── src/main.rs       # eBPF 程序入口
│   └── build.rs          # 使用 bpf-linker 编译
├── <app>/                # 用户态 loader (aya, std)
│   ├── src/main.rs       # 加载 eBPF bytecode，attach，读取 map
│   └── build.rs
├── prebuild.sh           # 交叉编译脚本
├── Cargo.toml            # workspace
└── qemu-<arch>.toml      # xtask 测试配置
```

### 2.2 现有 eBPF App 的实现模式总结

**已有 App**：

| App | 挂载方式 | Map 类型 | 输出 |
|-----|---------|---------|------|
| `net_stats` | kprobe (`#[inline(never)]`函数) | PerCpuArray | 计数器 |
| `sched_trace` | raw_tracepoint (`sched_switch`) | PerfEventArray | per-event 记录 |
| `rawtp` | raw_tracepoint (`sys_clone`) | aya_log | 日志 |
| `kret` | kretprobe | - | 返回值（受限） |

### 2.3 关键约束

**2.3.1 kretprobe 不可靠（sret ABI 问题）**

`net_stats/README.md` 明确指出：StarryOS 编译内联严重，kretprobe 函数返回值探针难以定位。

> "real root cause of net_stats zero byte counters: sret ptr is in RAX at kretprobe (old code read RDI), plus loader over-matches 19 symbols"

根因：Rust 的 `sret`（struct return）在 RISC-V/x86 等架构上，返回值指针在特定寄存器（如 x86 RAX），但内联导致实际寄存器分配不可预测，kretprobe 读取返回值变得不可靠。

**2.3.2 非原子计数失真**

对异步操作（如 poll task）的探针计数会产生非原子性失真。因为中断上下文和 task 上下文之间的竞争在单核上虽不会真正并发，但 eBPF 程序可能在任意点被中断，导致 per-CPU map 的增量在高频路径上出现竞争。

**2.3.3 推荐方案：tracepoint**

`net_stats/README.md` 结论：

> "相对可靠的方法是 trace"

即静态 tracepoint（`define_event_trace!`）— 它是编译器时确定、运行时 NOP 或直接函数调用的固定挂钩，无 breakpoint 重入风险，payload 由驱动主动写入，不依赖寄存器/栈解析。

**2.3.4 SG2002 特殊性**

- RISC-V 单核，无 BPF JIT，走 `rbpf` 解释器
- 无 BTF，无法使用 fentry/fexit（CO-RE 依赖 BTF）
- raw_tracepoint 依赖 StarryOS 的 `ktracepoint` 子系统
- kprobe 可用但需要 `#[inline(never)]` 标记目标函数

### 2.4 ktracepoint 机制

StarryOS 使用 `ktracepoint` crate（v0.6），提供类似 Linux tracepoint 的静态事件定义：

```rust
ktracepoint::define_event_trace!(
    sched_switch,
    TP_kops(crate::tracepoint::KernelTraceAux),
    TP_system(sched),
    TP_PROTO(prev_tid: u64, next_tid: u64, prev_state: u32),
    TP_STRUCT__entry { prev_tid: u64, next_tid: u64, prev_state: u32 },
    TP_fast_assign { prev_tid, next_tid, prev_state },
    TP_ident(__entry),
    TP_printk({ alloc::format!("prev_tid={} next_tid={} prev_state={}", ...) })
);
```

- 自动注册到 debugfs：`/sys/kernel/debug/tracing/events/<system>/<event>/id`, `enable`, `format`
- eBPF 通过 `raw_tracepoint` attach 到此事件
- raw_tracepoint context 为 `[u64; N]`（每个 TP_PROTO 字段扩展为 u64）

---

## 3. WiFi 驱动可监测点分析

### 3.1 数据面 TX 路径（关键热点）

基于 `sg2002-wifi.md` 报告和静态代码分析，TX 路径调用链：

```
wifi-tx poll task (sched-rr, 50ms time slice, cpu0)
  └─ tx_process(bus)
       └─ process_cmd_tx(bus)           ← CMD 优先
       └─ process_data_tx(bus)          ← DATA batch (最多 TX_BATCH_LIMIT=64 帧)
            └─ send_single_data_frame(bus, tx_frame)
                 ├─ build_data_frame()       ← Vec<u8> 堆分配 (~1536B)
                 ├─ check_data_flow_control() ← CMD52 读流控寄存器 (~5-10µs)
                 └─ transport.write_fifo()   ← SDIO 锁 → CMD53 write
                      └─ CviSdhci::write_fifo()
                           └─ cmd53_write_fixed()
                                ├─ cmd53_xfer()   ← wait_cmd_idle + send CMD53
                                ├─ pio_write()    ← 3 blocks × 128 MMIO writes
                                └─ wait_transfer_complete()
                                     └─ poll_int_status()  ← Phase1 忙等(3ms) or Phase2 yield
```

### 3.2 数据面 RX 路径

```
SDIO CARD_INT (IRQ#38) → sdhci1_irq_handler
  └─ irq_waker.wake()  → wifi-rx poll task 被唤醒
       └─ process_rx_frames(bus)
            └─ read_fifo_data()          ← SDIO 锁 → CMD53 read
            └─ build_and_enqueue_eth_frame()
            └─ invoke_rx_data_callback() → 唤醒 net-poll worker
```

### 3.3 已识别的关键监控指标

基于优化报告的经验和 netstacklat 的分层诊断思想：

| 指标 | 测量点 | 诊断价值 | 优化报告关联 |
|------|--------|---------|-------------|
| **SDIO 传输耗时** | `wait_transfer_complete` 前后 | 发现 SDIO 总线异常慢/卡死；量化 PIO vs DMA 收益 | Part 1 根因 (48ms→212µs) |
| **流控阻塞时间** | `check_data_flow_control` 前后 | 量化固件排空瓶颈 | Part 2 §10.4（流控空转修复） |
| **Phase2 掉入率** | `poll_int_status` | 发现忙等窗口不足、调度抖动 | Part 1 根因 (phase2_iters 恒为 1) |
| **CMD53 块大小分布** | `cmd53_write_fixed` 入口 | 评估多帧拼包收益 | DMA/拼包优化 |
| **A-MPDU 聚合深度** | `send_single_data_frame` 出口 | 验证 HE/HT 聚合是否生效 | Part 2 §10.1（HT 对齐修复） |
| **TX/RX 帧速率** | `tx_process`/`process_rx_frames` 入口 | 端到端吞吐、协议分布 | 基准测试 |
| **SDIO 锁持有时长** | `write_fifo`/`read_fifo` 入口→出口 | 量化 TX/RX 锁争用 | Part 1 RX 饥饿 (6s 仅 3 帧) |
| **TX batch size** | `process_data_tx` 循环计数 | 评估 CMD 优先中断频率 | Part 2 CMD pending_flag 队头阻塞 |
| **kicker 唤醒频率** | TX-kick/RX-kick poll task | 判断事件驱动是否可靠 | Part 2 §10.2 (kicker 10ms→1ms) |
| **RX 中断到处理延迟** | ISR→rx task 被调度 | IRQ 延迟 | 报告未涉及但基础指标 |

### 3.4 probe point 类型选择

| 监测点 | 推荐方案 | 原因 |
|--------|---------|------|
| SDIO poll/wait 耗时 | **tracepoint** + 手动打时间戳 | `poll_int_status` 在锁内、IRQ 可能被屏蔽时执行，kprobe 在此上下文有重入风险 |
| 流控检查 | **tracepoint** | `yield_now()` 在调度路径上，kprobe 可能触发重入 |
| 帧发送/接收计数 | **kprobe** 或 tracepoint | kprobe 可行（入口参数即可，不需要返回值），参考 net_stats 先例加 `#[inline(never)]` |
| 锁持有时长 | **tracepoint** | 需要 entry→exit 配对，kretprobe 不可靠 |
| TX batch size | kprobe (入口) 或 tracepoint | 只需读入口参数 |

**推荐策略**：以 tracepoint 为主、kprobe 为辅。热路径（SDIO 等待、锁、调度点）用 tracepoint 消除重入风险；低风险路径（帧发送入口）可用 kprobe 降低代码侵入性。

---

## 4. 方案设计：基于 tracepoint 的 WiFi 性能 eBPF 工具

### 4.1 核心设计思想

借鉴 netstacklat 的三层思路，适配 StarryOS WiFi 驱动的特点：

1. **时间戳载体**：netstacklat 依赖 Linux `skb->tstamp`，StarryOS 没有此基础设施。替代方案：在**关键路径入口手动记录单调时钟时间戳**，通过 tracepoint payload 传递给 eBPF 程序。不维护 per-packet 状态（避免 eBPF map 查找开销），而是利用 tracepoint 的 payload 字段传递 timestamp。

2. **分层测量**：沿 WiFi TX/RX 数据面在多个关键点设置 tracepoint，每个 tracepoint 携带统一的 `ts_ns: u64` 字段。eBPF 程序使用 `bpf_ktime_get_ns() - ts_ns` 计算延迟，写入 index-2 histogram。

3. **指数直方图输出**：直接复刻 netstacklat 的 base-2 histogram 方案（34 bucket），使用 `BPF_MAP_TYPE_PERCPU_HASH`。

### 4.2 架构总览

```
┌─────────────────────────────────────────────────────────────────┐
│ WiFi Driver (components/aic8800 + components/sdhci-cv1800)     │
│                                                                 │
│  关键路径入口手动 tracepoint:                                    │
│                                                                 │
│  trace_sdio_xfer_start(nbytes, dir)     → ts_ns written to ringbuf│
│  trace_sdio_xfer_done(nbytes, dir)      → ts_ns included        │
│  trace_wifi_tx_frame(len, vif_idx)                              │
│  trace_wifi_rx_frame(len)                                        │
│  trace_wifi_flow_ctrl(fc_value)                                  │
│  trace_wifi_tx_batch(n_frames)                                    │
│                                                                 │
│  备选 kprobe (入口参数):                                         │
│  kprobe:send_single_data_frame → arg(0) = &self (TxFrame info) │
│  kprobe:process_rx_frames → entry count                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ eBPF Program (aya-ebpf, riscv64)                                │
│                                                                 │
│  raw_tracepoint/sdio_xfer_done:                                 │
│    latency = bpf_ktime_get_ns() - args[0] (ts_ns)              │
│    → increment_exp2_histogram(&SDIO_XFER_LATENCY, key, latency) │
│                                                                 │
│  raw_tracepoint/wifi_tx_frame:                                  │
│    → increment counter                                          │
│    → record frame size distribution                             │
│                                                                 │
│  Maps:                                                          │
│    PERCPU_HASH: latency histogram (key={hook, bucket})          │
│    PERCPU_ARRAY: counters (tx_pkts, rx_pkts, ...)              │
│    PERF_EVENT_ARRAY: event stream (for detailed per-event)      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Userspace Loader (aya, StarryOS userspace)                      │
│                                                                 │
│  - Load embedded eBPF bytecode                                  │
│  - Attach to tracepoints via raw_tracepoint                     │
│  - Periodic read: sum per-CPU histogram buckets → print        │
│  - Output: console exp2 histograms, Prometheus endpoint         │
└─────────────────────────────────────────────────────────────────┘
```

### 4.3 Tracepoint 定义

在 `components/sdhci-cv1800/src/` 和 `components/aic8800/src/` 中定义以下 tracepoint：

#### SDIO 总线层 (`wifi:sdio_*`)

```rust
// --- sdio_xfer ---
// 在 cmd53_write_fixed / cmd53_read_fixed 入口
ktracepoint::define_event_trace!(
    sdio_xfer_start,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(nbytes: u32, dir: u8, ts_ns: u64),
    TP_STRUCT__entry { nbytes: u32, dir: u8, ts_ns: u64 },
    TP_fast_assign { nbytes, dir, ts_ns },
    TP_ident(__entry),
    TP_printk({ alloc::format!("sdio_xfer_start nbytes={} dir={}", __entry.nbytes, __entry.dir) })
);

// --- sdio_xfer_done ---
// 在 cmd53_write_fixed / cmd53_read_fixed 出口（传输完成，中断已确认）
ktracepoint::define_event_trace!(
    sdio_xfer_done,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(nbytes: u32, dir: u8, poll_us: u32, phase2: u8, ts_ns: u64),
    TP_STRUCT__entry { nbytes: u32, dir: u8, poll_us: u32, phase2: u8, ts_ns: u64 },
    TP_fast_assign { nbytes, dir, poll_us, phase2, ts_ns },
    TP_ident(__entry),
    TP_printk({ alloc::format!("sdio_xfer_done nbytes={} dir={} poll_us={} phase2={}", ...) })
);
```

#### WiFi 数据面 (`wifi:tx_frame`, `wifi:rx_frame`, `wifi:flow_ctrl`)

```rust
// --- wifi_tx_frame ---
// 在 send_single_data_frame 入口
ktracepoint::define_event_trace!(
    wifi_tx_frame,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(frame_len: u32, vif_idx: u8, is_mgmt: u8, fc_value: u8, ts_ns: u64),
    TP_STRUCT__entry { frame_len: u32, vif_idx: u8, is_mgmt: u8, fc_value: u8, ts_ns: u64 },
    TP_fast_assign { frame_len, vif_idx, is_mgmt, fc_value, ts_ns },
    TP_ident(__entry),
    TP_printk({ alloc::format!("wifi_tx_frame len={} vif={} mgmt={} fc={}", ...) })
);

// --- wifi_rx_frame ---
// 在 process_rx_frames 入口（每帧处理时）
ktracepoint::define_event_trace!(
    wifi_rx_frame,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(frame_len: u32, ts_ns: u64),
    TP_STRUCT__entry { frame_len: u32, ts_ns: u64 },
    TP_fast_assign { frame_len, ts_ns },
    TP_ident(__entry),
    TP_printk({ alloc::format!("wifi_rx_frame len={}", __entry.frame_len) })
);

// --- wifi_tx_batch ---
// 在 process_data_tx 循环出口
ktracepoint::define_event_trace!(
    wifi_tx_batch,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(n_frames: u32, interrupted_by_cmd: u8),
    TP_STRUCT__entry { n_frames: u32, interrupted_by_cmd: u8 },
    TP_fast_assign { n_frames, interrupted_by_cmd },
    TP_ident(__entry),
    TP_printk({ alloc::format!("wifi_tx_batch n={} interrupted={}", ...) })
);
```

#### tx.rs poll 循环层 (`wifi:poll_*`)

```rust
// --- wifi_poll_cycle ---
// 在 tx_process 结束（或 poll task 每次循环）时
ktracepoint::define_event_trace!(
    wifi_poll_cycle,
    TP_kops(KernelTraceAux),
    TP_system(wifi),
    TP_PROTO(thread: u8, did_work: u8, n_tx_cmd: u32, n_tx_data: u32, n_rx: u32),
    TP_STRUCT__entry { thread: u8, did_work: u8, n_tx_cmd: u32, n_tx_data: u32, n_rx: u32 },
    TP_fast_assign { thread, did_work, n_tx_cmd, n_tx_data, n_rx },
    TP_ident(__entry),
    TP_printk({ alloc::format!("wifi_poll_cycle thread={} tx_c={} tx_d={} rx={}", ...) })
);
```

### 4.4 eBPF Map 设计

```rust
// --- histogram map (PERCPU_HASH) ---
// key = {hook: u16, bucket: u16}
// value = u64 count (per-CPU)
struct HistKey {
    hook: u16,    // enum: SDIO_XFER_DONE=1, TX_FRAME=2, RX_FRAME=3, ...
    bucket: u16,  // log2 histogram bucket index (0..34)
}

// --- counter map (PERCPU_ARRAY) ---
// index → u64 value (per-CPU summed by loader)
enum CounterSlot {
    TX_PKTS = 0,
    TX_BYTES = 1,
    RX_PKTS = 2,
    RX_BYTES = 3,
    TX_MGMT_PKTS = 4,
    TX_DATA_PKTS = 5,
    FLOW_CTRL_BLOCKS = 6,
    SDIO_PHASE2_DROPS = 7,
}
```

### 4.5 eBPF 程序设计

```rust
// wifi_tx_frame probe: 计数 + 帧大小分布
#[raw_tracepoint(tracepoint = "wifi_tx_frame")]
fn wifi_tx_frame_probe(ctx: RawTracePointContext) -> i32 {
    let args = unsafe { &*(ctx.as_ptr() as *const [u64; 5]) };
    let frame_len = args[0] as u32;
    let is_mgmt = args[2] as u8;
    let fc_value = args[3] as u8;
    let ts_ns = args[4];

    // 帧计数器
    if is_mgmt != 0 {
        add_to(TX_MGMT_PKTS, 1);
    } else {
        add_to(TX_DATA_PKTS, 1);
    }
    add_to(TX_PKTS, 1);
    add_to(TX_BYTES, frame_len as u64);

    // 流控信用值分布
    record_value(FC_VALUE_HIST, fc_value as u64);

    0
}

// sdio_xfer_done probe: SDIO 传输延迟
#[raw_tracepoint(tracepoint = "sdio_xfer_done")]
fn sdio_xfer_done_probe(ctx: RawTracePointContext) -> i32 {
    let args = unsafe { &*(ctx.as_ptr() as *const [u64; 5]) };
    let nbytes = args[0] as u32;
    let dir = args[1] as u8;
    let poll_us = args[2] as u32;
    let phase2 = args[3] as u8;
    let ts_ns = args[4];

    // 传输总延迟 (entry ts_ns → now)
    let now = unsafe { bpf_ktime_get_ns() };
    let latency = now.saturating_sub(ts_ns);

    let key = HistKey { hook: if dir == 0 { SDIO_TX_LATENCY } else { SDIO_RX_LATENCY }, bucket: 0 };
    record_latency(latency, &key);

    // Phase2 掉入计数
    if phase2 != 0 {
        add_to(SDIO_PHASE2_DROPS, 1);
    }

    0
}
```

### 4.6 与 netstacklat 的差异对照

| 维度 | netstacklat (Linux) | WiFi Monitor (StarryOS) |
|------|---------------------|------------------------|
| **时间戳来源** | `skb->tstamp` (kernel 自动打) | 驱动手动 `monotonic_time_nanos()` 通过 tracepoint payload 传递 |
| **Probe 类型** | fentry/fexit (BTF CO-RE) | raw_tracepoint (ktracepoint) + 可选 kprobe |
| **延迟定义** | `now - skb_tstamp` (跨层) | `now - entry_ts` (同一 probe 的 entry→exit) |
| **覆盖范围** | IP→TCP→socket→app 整个收包路径 | SDIO→固件→驱动→网络栈 WiFi 收发包 |
| **输出格式** | exp2 histogram | exp2 histogram (复用 bits.bpf.h log2l) |
| **过滤能力** | PID, ifindex, cgroup, netns | 不支持（StarryOS 无对应基础设施） |
| **Filter** | 是（通过 volatile const config） | 简化版（tracepoint enable/disable 即可） |
| **Map 类型** | PERCPU_HASH histogram | PERCPU_HASH histogram + PERCPU_ARRAY counters |
| **语言** | C (libbpf) | Rust (aya-ebpf) |
| **架构** | x86_64 (JIT) | riscv64 (rbpf 解释器) |

### 4.7 关键设计决策

#### 4.7.1 为什么不链式测量（entry→exit 跨 probe）？

netstacklat 用 `skb->tstamp` 避免了 per-packet 状态跟踪。StarryOS 没有此基础设施，替代方案有：

- **方案 A（选中）**：每个 tracepoint 自带 `ts_ns`，eBPF 算 `now - ts_ns`。entry probe 打时间戳，exit probe 带时间戳。这是**可靠且简单**的方案。
- **方案 B（不选）**：entry probe 写 per-packet 状态 map (key=packet_id)，exit probe 查找。问题：(1) 驱动侧需要分配 packet_id；(2) eBPF map 查找在 rbpf 解释器下开销大；(3) map 清理复杂。

**方案 A 的代价**：每个 tracepoint 获得的是"从 entry 到 exit 这个单段延迟"，不能自动链式汇总。但对于 WiFi 驱动监测来说足够了——**性能分析的重点是识别单段瓶颈（SDIO wait、流控被阻塞、锁争用）**，而非跨层累积延迟。

#### 4.7.2 为什么不用 Histogram 做 entry→exit 跨帧？

对于**需要跨帧追踪**的场景（如"SDIO xfer 入口→出口延迟"），我们可以在 entry tracepoint 的 payload 中写入 `ts_ns`，然后在 exit tracepoint payload 中也带同一个 `ts_ns`（即 entry 的 timestamp），eBPF 在 exit 侧计算 `now - ts_ns`。关键在于 `ts_ns` 由**驱动侧**写入 tracepoint payload——eBPF 无需维护 per-event 状态。

#### 4.7.3 kprobe 辅助计数

对于纯计数（不需要 entry/exit 配对），kprobe `#[inline(never)]` 标记 + 入口参数读取是经济的选择。参考 net_stats 的做法：`DeviceHandle::count_tx/count_rx` 加 `#[inline(never)]`。WiFi 驱动中可以：

- `send_single_data_frame` 加 `#[inline(never)]` → kprobe 读入参 `tx_frame: &TxFrame`
- `process_rx_frames` 入口计数（不需要返回值）

但需要权衡：`#[inline(never)]` 在热路径上**增加函数调用开销**（RISC-V 上约 10-20 instruction overhead）。对 1500fps 路径影响很小（报告显示 TX 线程 busy 仅 12-14%），但需要在 kprobe 和 tracepoint 之间按路径热度做合理选择。

---

## 5. 实现计划

### 5.1 阶段划分

#### 阶段 1：Tracepoint 埋点（1-2 天）

1. 在 `components/sdhci-cv1800/` 中添加 SDIO tracepoint：
   - `wifi:sdio_xfer_start` + `wifi:sdio_xfer_done`（在 `cmd53_write_fixed` 入口/出口）
   - `wifi:poll_int_status`（在 `poll_int_status` 入口/出口，含 phase2 标志）
   - 依赖 ktracepoint crate

2. 在 `components/aic8800/` 中添加数据面 tracepoint：
   - `wifi:tx_frame`（`send_single_data_frame` 入口）
   - `wifi:rx_frame`（`process_rx_frames` 入口）
   - `wifi:tx_batch`（`process_data_tx` 循环出口）
   - `wifi:poll_cycle`（`tx_process` 出口）

#### 阶段 2：eBPF 程序（2-3 天）

1. 创建 `apps/starry/ebpf/wifi_monitor/` workspace（三层结构）
2. `wifi_monitor-common`：共享常量、enum、HistKey 结构体
3. `wifi_monitor-ebpf`：eBPF 程序
   - raw_tracepoint handler × 6
   - histogram map（PERCPU_HASH，复刻 netstacklat log2l）
   - counter map（PERCPU_ARRAY）
   - 生成 riscv64 eBPF bytecode
4. `wifi_monitor`：用户态 loader
   - 加载 bytecode → attach raw_tracepoint
   - 定时读取 map → print exp2 histogram
   - CLI flags：`--interval N`, `--once`, `--list-probes`, `--enable-probes`

#### 阶段 3：上板验证（1 天）

1. SG2002 上部署：编译 starry image（含 tracepoint 的 kernel + wifi driver）
2. 运行 wifi_monitor，iperf3 驱动流量
3. 验证关键指标：
   - SDIO xfer 延迟直方图（期望 ~200µs 众数，对比报告 Part 1 48ms）
   - Phase2 掉入率（期望 0，验证 3ms busy-wait 窗口有效）
   - TX frame rate ~ 1500fps
   - 流控 credit 值分布

#### 阶段 4：完善和文档（1 天）

1. 补充 README、使用说明
2. 编写 xtask 测试配置（qemu-riscv64.toml，但需要模拟 WiFi 流量... QEMU 限制）
3. 报告与 sg2002-wifi.md 中已知数据的交叉验证

### 5.2 关键文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `components/sdhci-cv1800/Cargo.toml` | 增加 ktracepoint 依赖 | tracepoint 定义 |
| `components/sdhci-cv1800/src/lib.rs` | 插入 tracepoint 调用（2-3 行/点） | cmd53_write_fixed, poll_int_status |
| `components/aic8800/Cargo.toml` | 增加 ktracepoint 依赖 | |
| `components/aic8800/src/fdrv/thread/tx.rs` | 插入 tracepoint 调用 | send_single_data_frame, tx_process |
| `components/aic8800/src/fdrv/thread/rx.rs` | 插入 tracepoint 调用 | process_rx_frames |
| `apps/starry/ebpf/wifi_monitor/Cargo.toml` | 新建 workspace | 3 个 member crate |
| `apps/starry/ebpf/wifi_monitor/wifi_monitor-common/src/lib.rs` | 新建 | 共享 enum/structs |
| `apps/starry/ebpf/wifi_monitor/wifi_monitor-ebpf/src/main.rs` | 新建 | eBPF program |
| `apps/starry/ebpf/wifi_monitor/wifi_monitor/src/main.rs` | 新建 | userspace loader |
| `apps/starry/ebpf/wifi_monitor/prebuild.sh` | 新建 | 交叉编译 riscv64 |

### 5.3 风险与缓解

| 风险 | 缓解 |
|------|------|
| tracepoint 在热路径增加 overhead | tracepoint 默认 disable 时是 NOP（static branch），enable 后才走函数调用。每个 tracepoint ~50-100ns |
| `rbpf` 解释器在 1.5kfps 下不能跟上 | 用 PERCPU_* map 减少锁开销。如果跟不上，降低 tracepoint 开启数量或采样率 |
| ktracepoint 依赖 crate 不在当前分支可用 | 检查 `feat/net-enhance` 分支中的 ktracepoint 版本和 API，确保兼容 |
| RISC-V eBPF target 不支持某些 aya-ebpf macros | 参考 sched_trace 已验证的 riscv64 编译配置 |

### 5.4 预期产出

完成后将提供一个名为 `wifi_monitor` 的 StarryOS eBPF 工具，功能包括：

1. **SDIO 传输延迟指数直方图**：PX/RX 方向分开展示，暴露 PIO 忙等异常
2. **Phase2 掉入率监控**：检测调度器时间片导致的延迟放大
3. **WiFi TX/RX 帧计数器和字节数**：与 iperf3 对照验证
4. **流控信用值分布**：量化固件缓冲区排空速率
5. **TX batch size 分布**：评估 CMD 优先中断对批处理的影响
6. **poll thread 活动统计**：TX/RX poll 循环的工作量

输出格式与 netstacklat 一致：每个 probe point 一个 exp2 histogram，每 N 秒刷新，直观展示延迟分布和长尾。

---

## 6. 使用场景详解：板载 StarryOS 启动后如何测试

### 6.1 整体流程

```
┌─────────────────────────────────────────────────────────────┐
│ 1. StarryOS 启动 (WiFi 驱动自动 init, SoftAP 或 STA 已连)    │
│ 2. 运行 wifi_monitor 后台采集 (tracepoint 默认 disabled)      │
│ 3. 开启需要的 probe → eBPF 开始记录 histogram                 │
│ 4. 另一终端运行 iperf3 驱动流量                                │
│ 5. wifi_monitor 每 N 秒打印 histogram，观察延迟分布            │
│ 6. Ctrl-C 停止，查看最终报告                                   │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 命令行接口

```bash
# 查看所有可用 probe
/usr/bin/wifi_monitor --list-probes

# 输出：
# available hooks:
#   sdio-xfer-latency: SDIO CMD53 transfer latency (entry→done), per direction
#   wifi-tx-frame:     WiFi TX frame counter + size distribution
#   wifi-rx-frame:     WiFi RX frame counter + size distribution
#   wifi-tx-batch:     TX batch size (frames per poll cycle)
#   wifi-flow-ctrl:    flow-control credit value histogram
#   wifi-poll-cycle:   poll thread activity (work done per cycle)
```

```bash
# 开启所有 probe，每 5 秒打印一次（默认）
/usr/bin/wifi_monitor

# 只开 SDIO 延迟和流控监控，2 秒刷新
/usr/bin/wifi_monitor --enable-probes sdio-xfer-latency,wifi-flow-ctrl --interval 2

# 单次快照（适合脚本采集）
/usr/bin/wifi_monitor --once --interval 0

# 关闭某个 probe（如只关 tx-frame）
/usr/bin/wifi_monitor --disable-probes wifi-tx-frame
```

### 6.3 典型测试场景

#### 场景 A：验证 SDHCI busy-wait 修复效果

**目的**：确认 `poll_int_status` Phase1 3ms 忙等是否生效，Phase2 掉入率是否为 0。

**步骤**：

```bash
# 终端 1：启动 monitor，只关注 SDIO 延迟
/usr/bin/wifi_monitor --enable-probes sdio-xfer-latency --interval 2

# 终端 2：打 iperf3 TCP 上行流量
iperf3 -c 192.168.50.100 -t 30 -i 1
```

**预期输出**（修复后）：
```
Sat Jul 25 10:30:00 2026
sdio-xfer-latency, dir=TX:
[    0.0us,    1.0us]       0 |
(    1.0us,    2.0us]       0 |
...
(  128.0us,  256.0us]   18423 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
(  256.0us,  512.0us]     127 |@                                           |
(  512.0us,  1.0ms  ]       3 |                                            |
(  1.0ms,   2.0ms  ]       0 |
...
( 16.0ms,  32.0ms  ]       0 |                ← 没有 Phase2 48ms 延迟!
count: 18553, average: 212.3us
```

**关键观察**：
- 众数在 `(128µs, 256µs]` bucket → 硬件传输 ~212µs（与报告 Part 1 数据吻合）
- `16ms` 以上 bucket 全零 → 无 sched-rr 50ms 时间片惩罚
- 如果出现 `(32ms, 64ms]` 有计数 → Phase2 掉入，可能是流控不足时的 yield

**如果修复未合入**（当前 `feat/net-enhance` 分支）的预期输出：
```
sdio-xfer-latency, dir=TX:
(  128.0us,  256.0us]       5 |                                            |
...
( 32.0ms,  64.0ms ]     18423 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|  ← 48ms!
count: 18428, average: 47.8ms
```

#### 场景 B：流控瓶颈诊断

**目的**：量化 flow control 触发频率和阻塞程度。报告 Part 2 §10.4 发现 50MHz 下 FC 阻塞占总窗口 40-54%。

**步骤**：

```bash
/usr/bin/wifi_monitor --enable-probes wifi-flow-ctrl,sdio-xfer-latency --interval 2
```

**预期输出**：
```
wifi-flow-ctrl:
(     0,      1]     320 |@@@@                                        |
(     1,      2]     892 |@@@@@@@@@@@@                                |
(     2,      3]   12450 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|  ← 频繁 credit=2
(     3,      4]    2100 |@@@@@@@                                     |
(     4,      8]     230 |                                            |
count: 15992, average: 2.4

sdio-xfer-latency, dir=TX:
(  128.0us,  256.0us]   11000 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
(  256.0us,  512.0us]     300 |@                                           |
(  512.0us,  1.0ms  ]     120 |                                            |
...
( 64.0ms, 128.0ms  ]      15 |                                            |  ← 偶发长延迟 (流控等待)
count: 11535, average: 345.2us
```

**诊断**：
- credit 大量集中在 2 → 固件 buffer 排空跟不上 host 灌入速度 → 需要 DMA + 多帧拼包
- SDIO 延迟偶有 64ms+ 尖峰 → `check_data_flow_control` 的 yield 路径触发了调度惩罚

#### 场景 C：端到端吞吐对照

**目的**：用 eBPF 计数验证 iperf3 报告的吞吐，逐层对比找出损耗。

**步骤**：

```bash
# 终端 1：全开
/usr/bin/wifi_monitor --interval 5

# 终端 2
iperf3 -c 192.168.50.100 -t 30
```

**预期输出**（结合 counter map）：
```
=== WiFi Monitor Counters (5s interval) ===
tx_pkts:    5710     tx_bytes:    8201540   (~13.1 Mbps)
rx_pkts:    3842     rx_bytes:     326570   (mostly TCP ACKs)
tx_data:    5705     tx_mgmt:          5    (beacon/keepalive)
fc_blocks:     0     phase2_drops:     0
tx_batch_avg: 8.3   poll_cycles:    712
```

**对照链条**：

```
eBPF tx_bytes (5s) → 8.2 MB → ~13.1 Mbps   ← 应该与 iperf3 sender 报告接近
iperf3 sender       → ~13.7 Mbps            ← 报告 Part 2 终点数据
差距 ≈ 4% → 合理 (eBPF 统计 L2 帧长含协议头，iperf3 统计 TCP payload)
```

#### 场景 D：TX/RX 锁争用检测

**目的**：量化单一 `Mutex<dyn SdioHost>` 导致的 TX/RX 互斥。报告 Part 1 记录 TX 满载时 RX 6 秒仅收 3 帧。

**步骤**：

```bash
# 同时打上行 + 下行流量
# 终端 1
/usr/bin/wifi_monitor --enable-probes wifi-tx-frame,wifi-rx-frame,sdio-xfer-latency --interval 1

# 终端 2：上行
iperf3 -c 192.168.50.100 -t 30 &

# 终端 3：下行
iperf3 -c 192.168.50.100 -t 30 -R &
```

**分析方法**：

```
# 每秒采样一次，观察 TX 秒级吞吐和 RX 秒级吞吐的反向关系
t=1s:  tx=1420fps  rx=   3fps  → RX 被饿死！
t=2s:  tx=1450fps  rx=   5fps
t=3s:  tx=1380fps  rx=   8fps
```

如果 RX fps 在 TX 满载时塌陷到个位数，说明锁争用严重——需要 SDIO 锁拆分或 DMA（DMA 传输期间可释放锁）。

### 6.4 部署流程

#### 编译和打包

```bash
# 编译 starry image（含 tracepoint + eBPF app）
cargo xtask starry app board --arch riscv64 \
    --test-case ebpf/wifi_monitor \
    --board licheerv-nano-sg2002

# 产物：
#   os/StarryOS/target/riscv64gc-unknown-linux-musl/release/starry
#   包含：
#     - StarryOS kernel (含 sdhci-cv1800 + aic8800 tracepoint)
#     - /usr/bin/wifi_monitor (static musl binary, eBPF bytecode 内嵌)
```

#### 板载启动后操作

```bash
# 1. 确认 WiFi 已连接
cat /proc/net/dev           # 应看到 wlan0
# 或使用 wifi_switch 工具
wifi_switch sta MyAP passphrase

# 2. 确认 tracepoint 存在
ls /sys/kernel/debug/tracing/events/wifi/
# 应输出: sdio_xfer_start  sdio_xfer_done  tx_frame  rx_frame  tx_batch  poll_cycle

# 3. 直接用 trace_pipe 看原始事件 (不用 eBPF，先确认 tracepoint 有输出)
echo 1 > /sys/kernel/debug/tracing/events/wifi/sdio_xfer_done/enable
cat /sys/kernel/debug/tracing/trace_pipe
# 应看到类似:
#   wifi-tx-1234 [000] ...: sdio_xfer_done: nbytes=1536 dir=0 poll_us=212 phase2=0

# 4. 关闭 raw trace，启动 eBPF
echo 0 > /sys/kernel/debug/tracing/events/wifi/sdio_xfer_done/enable
/usr/bin/wifi_monitor --enable-probes sdio-xfer-latency,wifi-tx-frame --interval 2

# 5. 另一终端或手机打流量
iperf3 -c 192.168.50.100 -t 30 -i 1
```

### 6.5 输出格式详解

每个 probe point 的 exp2 histogram 输出：

```
sdio-xfer-latency, dir=TX:          ← probe 名称 + 维度 label
[    0.0us,    1.0us]       0 |     ← bucket 0: [0, 1]
(    1.0us,    2.0us]       0 |     ← bucket 1: (1, 2]
(    2.0us,    4.0us]       0 |     ← bucket 2: (2, 4]
...
(  128.0us,  256.0us]   18423 |@@@@@@@@@@@@@@@@@@@@@@@@|  ← bucket 8: (128, 256]
(  256.0us,  512.0us]     127 |                            ← bucket 9: (256, 512]
...
(   8.0ms,   16.0ms]        0 |
(  16.0ms,   32.0ms]        3 |                            ← 少量慢路径!
...
(   8.0s,    17.2s ]        0 |    ← bucket 33 (last): 溢出桶
count: 18553, average: 235.7us  ← 总计 + 平均值
```

注：`[` 表示闭区间（含下界），`(` 表示开区间（不含下界）。第一个 bucket 是 `[0, 1]`（包含 0），其他都是 `(lower, upper]`。

Bar 长度按最大 bucket 归一化，**不是绝对比例**——所以用数值 + bar 结合看绝对量级。

### 6.6 与 trace_pipe 的配合：两阶段诊断法

eBPF 擅长**聚合统计**（histogram/counter），但不适合**逐事件追踪**（per-packet 输出会淹没解释器）。配合 trace_pipe 做两阶段诊断：

**阶段 1：宏观定位（eBPF histogram）**

```bash
/usr/bin/wifi_monitor --enable-probes sdio-xfer-latency --interval 2
# 观察延迟分布 → 发现异常 bucket（如有 48ms 众数）→ 锁定问题方向
```

**阶段 2：微观抓取（trace_pipe）**

```bash
# 只对问题事件开 trace，抓几十条记录细看
echo 1 > /sys/kernel/debug/tracing/events/wifi/sdio_xfer_done/enable

# 加 filter：只看 poll_us > 10000 的异常事件
echo "poll_us > 10000" > /sys/kernel/debug/tracing/events/wifi/sdio_xfer_done/filter

cat /sys/kernel/debug/tracing/trace_pipe | head -50
# 每条记录含 poll_us、phase2 标志，用于定性分析
```

这复刻了报告 Part 1 中的诊断手法："逐事件、带时间戳、按阈值门控的计时"，但这次走的是 eBPF 基础设施而非手工 `log::info!`。

### 6.7 自动化回归：CI 脚本示例

```bash
#!/bin/sh
# wifi_perf_regression_test.sh
# 板载自动回归：每次驱动改动后运行此脚本，验证无性能退化

set -e

# 1. 确保 WiFi 已连
wifi_switch sta TestAP testpass || { echo "FAIL: WiFi connect"; exit 1; }

# 2. 启动 monitor（后台）
/usr/bin/wifi_monitor --enable-probes sdio-xfer-latency,wifi-tx-frame \
    --interval 10 --once > /tmp/wifi_perf.txt &
MON_PID=$!

# 3. 打 15 秒 iperf3 TCP 上行
sleep 2  # 等 monitor attach 完成
iperf3 -c 192.168.50.1 -t 15 -i 0 > /tmp/iperf_tx.txt

# 4. 等 monitor 输出最后一轮
wait $MON_PID

# 5. 解析结果并断判
AVG_LATENCY=$(grep "sdio-xfer-latency.*dir=TX.*average" /tmp/wifi_perf.txt \
    | sed 's/.*average: \([0-9.]*\)us.*/\1/')
TX_MBPS=$(grep "sender" /tmp/iperf_tx.txt | tail -1 | awk '{print $7}')

echo "SDIO avg latency: ${AVG_LATENCY}us (threshold: <500us)"
echo "TCP TX throughput: ${TX_MBPS} Mbps (threshold: >10 Mbps)"

# Phase2 掉入率（从 counter map 读取）
PHASE2=$(grep "phase2_drops" /tmp/wifi_perf.txt | awk '{print $2}')
echo "Phase2 drops: ${PHASE2} (threshold: 0)"

# 判据
if [ "$(echo "$AVG_LATENCY > 500" | bc)" = "1" ]; then
    echo "FAIL: SDIO latency regression (${AVG_LATENCY}us > 500us)"
    exit 1
fi
if [ "$(echo "$TX_MBPS < 10" | bc)" = "1" ]; then
    echo "FAIL: TCP TX throughput regression (${TX_MBPS} < 10 Mbps)"
    exit 1
fi
if [ "$PHASE2" != "0" ]; then
    echo "FAIL: Phase2 yield detected (${PHASE2} drops)"
    exit 1
fi

echo "PASS: All WiFi perf regressions passed"
```

这个回归脚本的关键在于：**阈值是报告实测数据驱动的**（SDIO ~212µs 正常、Phase2 0 正常、TX >10M 正常），而非拍脑袋的值。

---

## 参考资料

- netstacklat source: `xdp-project/bpf-examples/netstacklat/`
- Red Hat article: https://developers.redhat.com/articles/2026/04/29/boosting-speed-use-ebpf-and-netstacklat-troubleshoot-latency
- StarryOS net_stats: `wt-feat-net-enhance/apps/starry/ebpf/net_stats/`
- StarryOS tracepoint: `os/StarryOS/kernel/src/tracepoint/`
- sg2002 WiFi report: `../sg2002-wifi.md`
