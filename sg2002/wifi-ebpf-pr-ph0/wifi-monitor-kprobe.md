# wifi_monitor — kprobe 版实现

**分支**: `sg2002-wifi-ebpf`
**架构**: kprobe/kretprobe 直接插桩 WiFi 驱动函数，无内核 tracepoint 中间层
**代码位置**: `apps/starry/ebpf/wifi_monitor/`

---

## 探针

共 4 组探针（6 个探针函数），全部通过 `/proc/kallsyms` 子串匹配符号名后 attach：

| 探针函数 | 类型 | 目标符号 | 目标函数 |
|---------|------|---------|---------|
| `tx_enqueue` | kprobe | `enqueue_data_frame` | `tx.rs:548` — TX 帧入队 |
| `sdio_write_entry` | kprobe | `write_fifo`+`sdio_transport`+`SdioTransport` | `sdio_transport.rs:166` — SDIO 写入口 |
| `sdio_write_return` | kretprobe | 同上 | 写返回点 |
| `sdio_read_entry` | kprobe | `read_fifo`+`sdio_transport`+`SdioTransport` | `sdio_transport.rs:156` — SDIO 读入口 |
| `sdio_read_return` | kretprobe | 同上 | 读返回点 |
| `irq_handler` | kprobe | `sdhci_irq_handler` | `irq.rs:78` — ISR 入口 |

kallsyms 消歧：`write_fifo`/`read_fifo` 使用三段子串（方法名 + 模块名 `sdio_transport` + 类型名 `SdioTransport`），确保不命中 `CviSdhci::write_fifo`/`CviSdhci::read_fifo`。其他符号唯一，单子串即可。

---

## Map

| Map | 类型 | Key | 内容 |
|-----|------|-----|------|
| `TX_FRAME` | u32→u64 | 0=入队次数, 1=以太网字节 | `enqueue_data_frame` 每次调用累积 |
| `SDIO_BYTES` | u32→u64 | 0=写字节, 1=读字节 | write/read_fifo 每次传输累积 |
| `SDIO_ERR` | u32→u64 | 0=write 失败, 1=read 失败 | kretprobe 中 `retval != 0` 时 +1 |
| `IRQ_CNT` | u32→u64 | 0=ISR 进入次数 | 含 base==0/status==0 提前返回路径 |
| `SDIO_WR_LATENCY` | u32→u64 | 0..7 八桶直方图 | write_fifo 延迟分布 |
| `SDIO_RD_LATENCY` | u32→u64 | 0..7 | read_fifo 延迟分布 |
| `WR_ENTRY` | u32→u64 | 0=入口 ns, 1=字节长度 | kprobe/kretprobe 配对暂存 |
| `RD_ENTRY` | u32→u64 | 0=ns, 1=len | 同上，读方向 |

分桶边界: `<300µs, <500µs, <1ms, <5ms, <20ms, <50ms, <100ms, >=100ms`。

---

## 数据流

```
[驱动调用]
    enqueue_data_frame(&Arc, Vec<u8>)
        │
        tx_enqueue kprobe ──→ 从 x1 隐藏指针偏移+16 读 Vec.len ──→ TX_FRAME

    write_fifo(&self, func, addr, &[u8])
        │
        sdio_write_entry  kprobe    ──→ WR_ENTRY ← ts, len
        sdio_write_return kretprobe ──→ now - entry_ts ──→ latency_bucket → SDIO_WR_LATENCY
                                     ──→ SDIO_BYTES(+len), SDIO_ERR(if retval != 0)

    read_fifo(&self, func, addr, &mut [u8])
        │
        sdio_read_entry  kprobe    ──→ RD_ENTRY ← ts, len
        sdio_read_return kretprobe ──→ now - entry_ts ──→ latency_bucket → SDIO_RD_LATENCY
                                   ──→ SDIO_BYTES(+len), SDIO_ERR(if retval != 0)

    sdhci_irq_handler(usize)
        │
        irq_handler kprobe ──→ IRQ_CNT +1
```

---

## Loader 流程

1. `setrlimit(RLIMIT_MEMLOCK, INFINITY)` — 解除 BPF map 内存限制
2. 加载编译时嵌入的 eBPF 字节码（build.rs 通过 aya-build 编译 `wifi-monitor-ebpf` crate）
3. 遍历 4 个目标符号，逐个从 `/proc/kallsyms` 解析实际符号名，然后 `program.load()` → `program.attach(symbol, 0)`
4. `loop { sleep(5s); dump_report() }` — 周期性读取所有 map 打印数字

输出格式：

```
=== wifi_monitor ===
TX_ENQUEUE  cnt=...  bytes=...
SDIO_WR      bytes=...  err=...
SDIO_RD      bytes=...  err=...
IRQ          cnt=...
SDIO_WR_LAT  [0] [1] [2] [3] [4] [5] [6] [7] (total=...)
SDIO_RD_LAT  [0] [1] [2] [3] [4] [5] [6] [7] (total=...)
```

---

## ABI 注意事项

`enqueue_data_frame` 的第二个参数 `eth_frame: Vec<u8>` 在 aarch64 Rust ABI 下是 24 字节聚合类型，走隐藏指针传参：`x1 = &Vec<u8>`。字段布局: `[x1+0]=ptr, [x1+8]=cap, [x1+16]=len`。当前从 `ctx.arg(1)` 取指针，加 16 偏移后用 `bpf_probe_read_kernel` 间接读 `len`。带 `0 < len <= 65536` 守卫。

`write_fifo`/`read_fifo` 的 `buf: &[u8]` 是胖指针（16 字节），在 aarch64 上拆为两个寄存器：`x3=buf.ptr, x4=buf.len`。直接从 `ctx.arg(4)` 取长度。

---

## 已知问题与 TODO

### WR_ENTRY 单槽竞争

`write_fifo` 至少有三个运行时调用方：

| 调用方 | 来源 | 线程 |
|--------|------|------|
| CMD/DATA/MGMT 发送 | `tx.rs:190,296,336` | TX 线程 |
| WPA2 EAPOL 握手 | `cmd.rs:439` → `api.rs:631/677` | WiFi 管理线程 |
| 固件下载 | `init.rs:304` | 一次性 |

关联握手期间 TX 线程持续发数据帧、管理线程发 EAPOL M2/M4，两线程并发进入 write_fifo。单槽 `WR_ENTRY` map 会被对方覆写：kretprobe 读到错误的时间戳，延迟计算错误；或被清导致样本丢失。**已删除旧的 WR_OVERLAP 计数器方案，待改为 per-thread 隔离（用 `bpf_get_current_pid_tgid()` 低 32 位做 entry map key）**。

`RD_ENTRY` 无此问题——运行时仅 RX 线程调用 `read_fifo`。

### SDIO_ERR 判别式依赖编译器布局

kretprobe 中 `retval != 0` 判定失败依赖于 aarch64 下 `Result<(), SdioError>` 的 Ok(()) 编码为 x0≈0。若编译器布局不同，错误计数可能偏零或偏高。未在真机实证。

### 计数器非原子

所有 map 操作均为 `get → +1 → insert` 读-改-写，非原子。多核或不可抢占探针间存在计数丢失。当前接受该精度（诊断用）。

### 无退出机制

`loop { sleep(5s) }` 无限循环，无 SIGTERM/SIGINT 处理。被 kill 时直接终止，无清理。

### Loader 无法区分读/写直方图桶含义

输出只打印 8 个数字，不标注各桶的微秒范围。需对照 `BUCKET_US_LIMITS` 常量解读。分桶边界在 common crate 定义，loader 和 eBPF 两侧均硬编码。

### kallsyms 首匹配

`resolve_symbol_name` 遍历 `/proc/kallsyms` 取第一个匹配行，无唯一性校验。当前符号集合不存在歧义，但如果同 crate 新增同名方法可能错配。
