# WiFi eBPF 性能监控实现评估

分支 `sg2002/wifi-ebpf`，基于 dev 分叉，含 2 个提交：

- `66c9a2f13` fix&test(sg2002,wifi): fix wifi boot & include sg2002 wifi config
- `588060224` feat(starry,aic8800): add eBPF-based WiFi driver performance monitor for sg2002

评估时间：2026-07-25

---

## 1. 架构总览

实现了一个四层架构的 WiFi 驱动 eBPF 性能监控系统：

```
┌──────────────────────────────────────┐
│  wifi_monitor (userspace loader)      │  ← aya 加载器，读取 maps 打印统计
│  wifi_monitor-ebpf (eBPF program)     │  ← raw_tracepoint 探针，写 maps
├──────────────────────────────────────┤
│  starry_kernel::tracepoint::wifi      │  ← 6 个 tracepoint 定义
│  (define_event_trace! + C ABI 包装)   │
├──────────────────────────────────────┤
│  axruntime::wifi_glue.rs              │  ← trait 实现，桥接 tracepoint
│  (ArceosSdioTrace / ArceosWifiTrace)  │
├──────────────────────────────────────┤
│  aic8800::trace (WifiTrace trait)     │  ← 驱动侧 trait 定义与调用点
│  sdhci-cv1800::trace (SdioTrace trait)│
└──────────────────────────────────────┘
```

**评价**：层次清晰，每层的职责边界明确。通过 trait + `AtomicPtr` 注入模式实现了驱动与 OS 的解耦，无 provider 时全部为零开销 no-op。

---

## 2. 各层详评

### 2.1 驱动侧 trace hooks

**文件**：`components/aic8800/src/trace.rs`, `components/sdhci-cv1800/src/trace.rs`

- `WifiTrace` trait 定义了 5 个 hook 方法：`now_nanos`, `on_tx_frame`, `on_rx_frame`, `on_tx_batch`, `on_poll_cycle`
- `SdioTrace` trait 定义了 3 个 hook 方法：`now_nanos`, `on_sdio_xfer_start`, `on_sdio_xfer_done`
- 通过 `AtomicPtr<&'static dyn Trait>` + `set_*` 安装，热路径上仅一次 `Acquire` load

**调用点覆盖**：

| Hook | 调用位置 | 状态 |
|------|---------|------|
| `on_sdio_xfer_start` | `sdhci-cv1800::cmd53_read_fixed`, `cmd53_write_fixed` | ✅ 正确覆盖了 CMD53 读/写两条路径 |
| `on_sdio_xfer_done` | 同上 | ✅ 携带 `ts_ns` 避免 eBPF 侧维护 per-event 状态 |
| `on_tx_frame` | `aic8800::tx::send_single_data_frame` | ✅ 每个数据帧和管理帧都触发 |
| `on_rx_frame` | `aic8800::rx::drain_func` | ⚠️ 见问题 1 |
| `on_tx_batch` | `aic8800::tx::process_data_tx` | ✅ 在 batch 结束后触发 |
| `on_poll_cycle` | `aic8800::tx::tx_process` | ✅ 在每次 poll 迭代结束后触发 |

**问题 1** — RX trace 时间点偏差：
`on_rx_frame` 在 `drain_func` 中 `read_fifo_data` 成功后、`dispatch_frames` 前调用。此时 `tr.now_nanos()` 获取的时间戳是 FIFO 读取完成后的时间，而非帧到达时间。对于评估 RX 路径延迟有轻微偏差（约 CMD53 传输耗时量级）。如需更精确的到达时间，应在 ISR 触发时取时间戳并沿调用链传递。

**问题 2** — `on_tx_frame` 的 `fc_value` 参数语义混乱：
```rust
// tx.rs:303
let fc = bus.transport.read_flow_ctrl_value().unwrap_or(0);
t.on_tx_frame(frame.data.len() as u32, vif_idx, frame.is_mgmt as u8, fc, now);
```
此处传入的 `fc` 是 SDIO 流控寄存器的值（flow control），而在 tracepoint 定义中该字段名为 `fc_value`（frame control value 之意）。在 eBPF 侧该字段也未被实际使用，但语义不一致可能导致后续维护困惑。建议重命名为 `flow_ctrl` 或传入实际的 802.11 frame control 字段。

### 2.2 内核 tracepoint 层

**文件**：`os/StarryOS/kernel/src/tracepoint/wifi.rs`

定义了 6 个 tracepoint，全部注册在 `wifi` 子系统下：

| Tracepoint | 参数 | 用途 |
|-----------|------|------|
| `sdio_xfer_start` | nbytes, dir, ts_ns | SDIO 传输开始 |
| `sdio_xfer_done` | nbytes, dir, poll_us, phase2, ts_ns | SDIO 传输完成+phase2 检测 |
| `wifi_tx_frame` | frame_len, vif_idx, is_mgmt, fc_value, ts_ns | TX 帧计数 |
| `wifi_rx_frame` | frame_len, ts_ns | RX 帧计数 |
| `wifi_tx_batch` | n_frames, interrupted_by_cmd | TX batch 统计 |
| `wifi_poll_cycle` | did_work, n_tx_cmd, n_tx_data | Poll 周期统计 |

通过 `#[no_mangle] extern "C"` 包装函数解决了 Rust 跨 crate 符号名不可靠的问题，使 `axruntime` 可以通过 FFI 可靠链接。

**评价**：tracepoint 选点合理，覆盖了 SDIO 数据路径和 WiFi TX/RX 数据平面的关键热点。`sdio_xfer_done` 携带入口时间戳 `ts_ns` 的设计避免了 eBPF 程序需要维护 per-event 状态，是好的做法。

**问题 3** — `TP_ident(__entry)` 用法：
tracepoint 定义中使用了 `TP_ident(__entry)`。需要确认这与 `ktracepoint` crate 的预期用法一致。如果 `__entry` 未被正确识别为 `TP_STRUCT__entry` 中定义的 struct，可能导致编译问题或 `TP_printk` 中引用 `__entry` 字段失败。

### 2.3 OS glue 层

**文件**：`os/arceos/modules/axruntime/src/wifi_glue.rs`

实现简洁，通过 `extern "C"` 声明引入内核 tracepoint 函数，并在 trait 实现中直接调用。`install_runtime()` 一次性安装所有 4 个 provider。

**评价**：无问题。`unsafe` 使用合理，tracepoint 函数为无状态 fire-and-forget，从任意上下文调用均安全。

### 2.4 eBPF 程序

**文件**：`apps/starry/ebpf/wifi_monitor/wifi_monitor-ebpf/src/main.rs`

**Maps**：
- `LATENCY_HIST`: `PerCpuArray<u64>`, 7×36=252 slots，存储每探针的延迟直方图
- `COUNTERS`: `PerCpuArray<u64>`, 6 slots，存储帧/字节计数器

**探针处理状态**：

| 探针 | 状态 | 说明 |
|------|------|------|
| `sdio_xfer_start` | 占位 | 仅 `return 0`，不记录数据 |
| `sdio_xfer_done` | 完整 | 计算延迟、写入直方图、检测 phase2 |
| `wifi_tx_frame` | 完整 | 递增 TX 计数器、按类型分类 |
| `wifi_rx_frame` | 完整 | 递增 RX 计数器 |
| `wifi_tx_batch` | 占位 | 仅 `return 0` |
| `wifi_poll_cycle` | 占位 | 仅 `return 0` |

**`log2l` 实现**：
```rust
fn log2l(v: u64) -> u16 {
    if v <= 1 { return 0; }
    let leading = v.leading_zeros();
    let pos = (63u32.wrapping_sub(leading)) as u16; // floor(log2)
    if v & (v - 1) != 0 { pos + 1 } else { pos }
}
```
使用 `leading_zeros` 实现的 ceil(log2)，无循环，在 riscv64 上编译为位操作指令。边界处理正确：`v=0,1 → 0`, `v=2 → 1`, `v=3 → 2`。

**问题 4** — `HIST_TOTAL` 的使用不一致：
在 eBPF 侧：
```rust
const HIST_SLOTS: u32 = N_PROBES * HIST_TOTAL as u32;
```
这里 `HIST_TOTAL = 36`（34 个正常 bucket + 2 个额外 slot），但 `record_latency` 中只写入 `bucket ∈ [0, HIST_NBUCKETS)`（即 0..34）。多出的 2 个 slot（overflow + sum）在 eBPF 侧从未被写入，仅在 userspace 读取时有特殊处理。这两个 slot 的含义应补充注释说明。

**问题 5** — 三个占位探针：
`sdio_xfer_start`、`wifi_tx_batch`、`wifi_poll_cycle` 目前仅 `return 0`，原因是它们的存在是为了让 `program.attach()` 成功（raw_tracepoint 需要探针函数存在才能 attach）。但这也意味着这三个 tracepoint 的 eBPF 数据收集功能完全未实现，当前仅通过 `debugfs trace_pipe` 提供文本输出。特别是 `wifi_tx_batch` 可以记录 batch size 分布，`wifi_poll_cycle` 可以统计 poll 效率。

### 2.5 Userspace loader

**文件**：`apps/starry/ebpf/wifi_monitor/wifi_monitor/src/main.rs`

- 支持 `--interval`, `--once`, `--list-probes`, `--enable-probes`, `--disable-probes`
- 加载 eBPF 字节码（通过 `aya::include_bytes_aligned!` 嵌入）
- 周期性读取 maps 并打印 exp2 直方图

**问题 6** — CLI 探针名称粒度不足：
`--enable-probes` 的 `parse_probe_name` 将 `"sdio-xfer-latency"` 映射为 `ProbeId::SdioTxLatency`。但 `ProbeId` 区分了 `SdioTxLatency=1` 和 `SdioRxLatency=2`。用户在 CLI 无法分别启用/禁用 TX 和 RX SDIO 延迟监控，因为 `enabled` 数组通过 `ProbeId::SdioTxLatency as usize`（=1）索引，而 `attach_probe` 中 `sdio_xfer_start` 和 `sdio_xfer_done` 都通过 `opts.enabled[ProbeId::SdioTxLatency as usize]` 控制。这意味着：
- 启用 probe 1（TX latency）同时也 attach 了两个 SDIO 探针
- RX latency（probe 2）的 `enabled` 标志未在任何 `attach_probe` 调用中使用
- 实际上 `sdio_xfer_start`/`sdio_xfer_done` 同时服务于 TX 和 RX，无法单独启用

**问题 7** — 直方图打印的 SI 单位后缀不一致：
`si_format` 对纳秒返回 `"n"`，微秒返回 `"u"`，毫秒返回 `"m"`，秒返回 `" "`（空格）。标准做法是秒级应显示 `" "` 并在数字后加 `"s"`，与后续代码 `writeln!(..., "{avg_str:.2} {avg_unit}s")` 一致。但纳秒/微秒/毫秒的后缀不包含 `"s"`（如 `"ns"`, `"us"`, `"ms"`），而格式化字符串追加了 `"s"`，导致输出如 `"123.45 ns"`（正确）和 `"1.23 ms"`（正确），但秒级显示为 `"1.23  s"`（多余空格）。这是个轻量级的展示问题。

### 2.6 构建系统

**文件**：`prebuild.sh`, `wifi_monitor/build.rs`, `wifi_monitor-ebpf/build.rs`

- `prebuild.sh`：交叉编译为静态 musl 二进制，安装到 StarryOS rootfs overlay
- 自动检测并安装 `bpf-linker`
- `build.rs`：通过 `aya_build::build_ebpf` 编译 eBPF 字节码并用 `include_bytes_aligned!` 嵌入

**问题 8** — `build.rs` 中的包发现方式脆弱：
```rust
let ebpf_package = packages
    .into_iter()
    .find(|cargo_metadata::Package { name, .. }| name.as_str() == "wifi_monitor-ebpf")
    .ok_or_else(|| anyhow!("wifi_monitor-ebpf package not found"))?;
```
通过遍历 `cargo metadata` 的所有包来查找 `wifi_monitor-ebpf`。这在 workspace 结构不变时工作正常，但如果 `wifi_monitor-ebpf` 被移出 workspace，会以清晰错误信息失败，这是可接受的。

### 2.7 QEMU 测试配置

**文件**：`qemu-riscv64.toml`

配置了 QEMU 启动后运行 `/usr/bin/wifi_monitor --list-probes`，验证输出含 `"available hooks:"`。这是一个基本的冒烟测试，确认 eBPF 程序能成功加载并列出探针。

---

## 3. 整体评估

### 优点

1. **架构设计优秀**：四层分离（驱动 hook → kernel tracepoint → eBPF → userspace）边界清晰，每层可独立演进
2. **解耦彻底**：通过 trait + `AtomicPtr` 注入，驱动核心不依赖任何 OS 或 eBPF 框架
3. **零开销默认**：无 provider 安装时全部 hook 为 no-op（单次 `Acquire` load + 虚函数调用开销极低）
4. **延迟追踪设计好**：`sdio_xfer_done` 携带入口时间戳，避免了 eBPF 侧维护 per-event 状态的复杂性
5. **直方图实现正确**：`log2l` 算法高效且边界处理正确，基于 `leading_zeros` 无循环
6. **跨 crate 链接可靠**：`#[no_mangle] extern "C"` 包装避免了 Rust 符号名在不同 crate 间不可靠的问题
7. **构建集成完整**：musl 静态编译、eBPF 字节码嵌入、QEMU 冒烟测试一应俱全

### 需要改进

| 优先级 | 问题 | 位置 |
|--------|------|------|
| 中 | RX trace 时间戳偏差（FIFO 读取后而非帧到达时） | `aic8800/rx.rs:328` |
| 中 | `on_tx_frame` 的 `fc_value` 实为 flow control 值 | `aic8800/tx.rs:303` |
| 中 | CLI 无法单独控制 TX/RX SDIO 探针启用 | `wifi_monitor/src/main.rs:87-98` |
| 低 | 三个占位探针未实现实际数据收集 | `wifi_monitor-ebpf/src/main.rs` |
| 低 | `HIST_TOTAL` 多出的 2 个 slot 缺少注释 | `wifi_monitor-common/src/lib.rs:9` |
| 低 | 秒级 SI 格式化多余空格 | `wifi_monitor/src/main.rs:239-249` |
| 低 | 缺少单元测试（`log2l`, histogram bucket 计算） | — |

### 功能完整度

| 功能 | 状态 |
|------|------|
| SDIO TX 延迟直方图 | ✅ 完整 |
| SDIO RX 延迟直方图 | ✅ 完整 |
| TX/RX 帧计数和字节统计 | ✅ 完整 |
| 管理帧 TX 分类计数 | ✅ 完整 |
| Phase2 slow path 检测 | ✅ 完整 |
| TX batch 大小统计 | ❌ 占位 |
| Poll cycle 效率统计 | ❌ 占位 |
| SDIO 传输大小分布 | ❌ 未实现 |
| eBPF map pinning（持久化） | ❌ 未实现 |

### 总结

这是一个架构良好、实现扎实的 WiFi 驱动 eBPF 性能监控系统。核心数据路径（SDIO 延迟 + TX/RX 帧统计）已完整实现并能正常工作。主要不足在于三个占位探针未实现数据收集功能，以及少数实现细节需要打磨。整体达到了可用的性能监控水平，适合在真实硬件上进行 WiFi 性能分析和瓶颈定位。
