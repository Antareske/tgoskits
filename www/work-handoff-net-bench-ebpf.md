# 当前工作交接记录

本文档记录当前分支已完成的工作、验证情况、已知问题和后续衔接建议，便于之后继续推进 StarryOS 网络性能测试与 eBPF `net_stats` 集成。

## 工作目标

本轮工作的主线是增强 `apps/starry/net-bench` 网络性能测试流程，并补充 eBPF 内核侧观测能力。

总体目标：

- 用 `iperf3` 建立 StarryOS 网络性能 smoke/baseline workflow。
- 支持 SLIRP、SMP、TAP 场景的多次测量和汇总。
- 记录环境指纹，避免裸数字缺少上下文。
- 引入 eBPF `net_stats`，在 Starry guest 内观测 ax-net TCP/UDP socket send/recv 路径。
- 将 eBPF 观测作为网络性能测试的辅助诊断信号，而不是替代 `iperf3` 的正式性能指标。

## net-bench 已完成内容

`apps/starry/net-bench` 当前设计为 StarryOS 网络性能基线入口，主要文件职责如下：

- `README.md`：说明测试场景、快速开始、结果汇总、环境指纹和常见问题。
- `run.sh`：host 侧入口，启动 host iperf3 server、运行 QEMU、保存日志、调用汇总。
- `net-bench-common.sh`：guest 侧公共 benchmark 逻辑，负责 warmup、多次测量、BEGIN/END marker。
- `net-bench.sh`：guest 侧 SLIRP 入口，设置 host IP 为 `10.0.2.2`。
- `net-bench-tap.sh`：guest 侧 TAP 入口，设置 host IP 为 `192.168.100.1`。
- `summarize.py`：host 侧解析 run log，输出 per-test mean/stddev 和 eBPF snapshot。
- `prebuild.sh`：准备 guest rootfs overlay，安装 iperf3 和脚本。

测试覆盖：

- `tcp1`：TCP 单流上行，guest -> host。
- `tcp4`：TCP 4 并发流上行。
- `tcp1r`：TCP 单流下行，host -> guest，使用 iperf3 reverse mode。
- `udp1g`：UDP 大包，目标 1 Gbit/s。
- `udp64`：UDP 64B 小包 PPS。

默认策略：每个 test-id 跑 1 次 warmup 和 5 次正式测量；`run.sh --repeat N` 支持跨 QEMU reboot 聚合，覆盖 cross-boot 方差。

`run.sh` 当前支持 `slirp`、`slirp-smp4`、`tap`、`all` 和 `--repeat N`，并会记录环境指纹到 `results/fingerprint-*.txt`，保存 run log 到 `results/starry-*.txt`，输出 summary 到 `results/summary-*.txt`。

`summarize.py` 当前支持解析 `NET_BENCH_BEGIN/NET_BENCH_END` 包裹的 iperf3 JSON，丢弃 warmup，输出 throughput/PPS/loss/retransmits 的 mean/stddev，并对相对标准差大于 10% 的结果标记 noisy。它也能解析 `NET_STATS_BEGIN/NET_STATS_END` 包裹的 eBPF snapshot 并输出 before、after、delta。

## eBPF net_stats 已完成内容

新增目录：

```text
apps/starry/ebpf/net_stats/
```

主要内容：

- `Cargo.toml`
- `Cargo.lock`
- `prebuild.sh`
- `qemu-x86_64.toml`
- `qemu-aarch64.toml`
- `qemu-riscv64.toml`
- `qemu-loongarch64.toml`
- `net_stats/`：userspace loader。
- `net_stats-ebpf/`：eBPF program。
- `net_stats-common/`：公共 crate，目前仅保留 `#![no_std]`。

当前观测 Starry ax-net socket 层：TCP send、TCP recv、UDP send、UDP recv。

输出 counter：

- `tcp_tx_pkts`
- `tcp_tx_bytes`
- `tcp_rx_pkts`
- `tcp_rx_bytes`
- `udp_tx_pkts`
- `udp_tx_bytes`
- `udp_rx_pkts`
- `udp_rx_bytes`

重要语义说明：`*_bytes` 是 socket API payload bytes；`*_pkts` 当前实际是 send/recv entry probe 命中次数，即 socket 调用次数，不是真实网络包数；不统计 TCP/IP/Ethernet header、wire bytes、重传、分片或 virtio queue 行为。

userspace loader 读取 `/proc/kallsyms`，用 Rust v0 mangled symbol fragment 匹配 ax-net 符号。同一类 send/recv 可能存在多个 monomorphized symbol，因此 loader 会把同一个 KProbe program attach 到所有匹配符号。

send byte 读取：当前观察到 `SocketOps::send` 返回 `AxResult<usize>` 时使用 sret pointer。x86_64 return site 中 `rax` 是返回结构指针，eBPF 读取 `+0` 的 discriminant 和 `+8` 的 byte payload，只在 `Ok` 时累计 bytes。

recv byte 读取：当前 x86_64 Starry QEMU 中观察到 byte count 位于 `rdx`，通过 aya x86_64 pt_regs 的 `ProbeContext::arg::<u64>(2)` 读取，并用 `MAX_IO_BYTES = 1 << 30` 过滤指针样异常值。

限制：该 ABI 解码仅在当前 x86_64 QEMU 上验证，aarch64、riscv64、loongarch64 不能默认复用该判断。

`net_stats` 支持：

- `--once`：attach 后立即输出一次 snapshot。
- `--test`：attach 后产生 TCP/UDP loopback 流量，再输出 snapshot。
- 默认周期模式：按 `--interval` 周期输出 snapshot，直到 Ctrl-C。

## eBPF 评估文档

已新增：

```text
apps/starry/net-bench/EBPF_NET_STATS.md
```

该文档覆盖实现目标、辅助观测定位、eBPF 探针设计、userspace loader 行为、byte 读取方式、输出格式、当前验证结果、是否达到观测要求、适用场景、稳定性和准确性评估、成熟度分级、推荐后续工作。

## 已完成验证

已运行 eBPF self-test：

```sh
cargo xtask starry app qemu -t ebpf/net_stats --arch x86_64
```

结果：

- QEMU 启动成功。
- Starry 中 `/proc/kallsyms` 可用。
- 能找到 Rust v0 mangled ax-net TCP/UDP send/recv 符号。
- kprobe/kretprobe attach 成功。
- `net_stats --test` 产生 TCP/UDP loopback 流量。
- 成功输出 `NET_STATS_END`。
- TCP/UDP tx/rx byte counter 均非零。

一次验证中的关键 byte counter：

```text
tcp_tx_bytes=192
tcp_rx_bytes=256
udp_tx_bytes=88
udp_rx_bytes=88
```

已运行：

```sh
cargo fmt --manifest-path apps/starry/ebpf/net_stats/net_stats/Cargo.toml
cargo fmt --manifest-path apps/starry/ebpf/net_stats/net_stats-ebpf/Cargo.toml
cargo clippy --manifest-path apps/starry/ebpf/net_stats/net_stats/Cargo.toml --all-targets
```

结果：fmt 通过，userspace loader clippy 通过。`cargo xtask clippy --package net_stats` 不适用，因为 `net_stats` 不是 workspace package。

测试中出现过以下非致命日志，不影响当前 kprobe/kretprobe 统计：

```text
bpf: unsupported command BPF_BTF_LOAD
bpf: unsupported command BPF_LINK_CREATE
bpf map type BPF_MAP_TYPE_CPUMAP not implemented
bpf map type BPF_MAP_TYPE_DEVMAP not implemented
failed to initialize eBPF logger: AYA_LOGS not found
```

## 当前问题和风险

### 1. net-bench 中 eBPF before/after 集成还不正确

`run.sh` 当前在 host 侧执行：

```sh
timeout 6 net_stats --once
```

这不能保证采样的是 Starry guest 内核。如果 host 上存在同名 `net_stats`，会采到 host 环境。正确方向应该是在 Starry guest 中执行 `/usr/bin/net_stats --once`，将输出纳入 QEMU guest log，再由 host 侧 `summarize.py` 解析 guest 输出。

### 2. before snapshot 可能被覆盖

`run.sh` 当前 before sampling 用 `tee -a "$result_file"` 追加，但随后 QEMU 输出使用 `tee "$result_file"`，会覆盖前面写入的 before snapshot。需要改为全程追加，或调整日志组织方式。

### 3. `*_pkts` 命名存在歧义

当前 `*_pkts` 统计的是 send/recv entry probe 命中次数，实际是 socket 调用次数，不是真实 packet count。后续建议改名为 `*_calls`，或在 README、summary 和 eBPF 文档中明确说明。

### 4. SMP 并发计数不严格

eBPF map 更新当前是 `*slot += delta`，不是原子更新。在 SMP、高并发、多 stream 场景下可能丢增量。后续建议使用 atomic add 或 Per-CPU map。

### 5. 跨架构 ABI 未验证

当前 byte decode 只在 x86_64 验证。send 的 sret pointer 位置、recv 的 byte count 寄存器、`AxResult<usize>` layout 都可能随架构或编译器变化。

### 6. eBPF 统计层级不是性能指标层级

`net_stats` 统计 socket 层 payload，不统计 TCP/IP/Ethernet header、wire bytes、virtio queue bytes、TCP retransmission wire impact、offload 或队列层行为，因此不能和 iperf3 throughput 做严格数值对齐。

## 当前成熟度判断

- net-bench workflow：基础流程已搭建，仍需真实场景复测和细节打磨。
- `summarize.py`：可用于当前 marker 格式日志。
- eBPF `net_stats` x86_64 self-test：可用。
- eBPF `net_stats` 作为 net-bench 辅助观测：核心 app 可用，自动集成仍需修。
- eBPF `net_stats` 作为正式性能指标：暂不建议。
- eBPF `net_stats` 跨架构工具：尚未完成。
- SMP 精确统计：尚未完成。

总体成熟度：实验性可用，适合继续迭代，不适合直接作为正式性能结论依据。

## 建议后续工作顺序

### P0：修正 net-bench 与 eBPF 集成

- 修改 guest 侧脚本，在 benchmark 前后执行 `/usr/bin/net_stats --once`。
- 或增加专门的 guest shell wrapper，把 before snapshot、benchmark、after snapshot 串在同一次 QEMU 执行中。
- 修正 `run.sh` 的日志写入，避免覆盖 before snapshot。
- 跑一次完整 QEMU net-bench，确认 summary 中 eBPF delta 来自 guest log。

### P1：修正文档和字段语义

- 在 `README.md` 中引用 `EBPF_NET_STATS.md`。
- 明确 `*_pkts` 是调用次数。
- 考虑把输出字段改为 `*_calls`。

### P2：SMP 计数修正

- 调研 aya-ebpf 中可用 atomic add 写法。
- 或改用 Per-CPU Array map。
- 对 `tcp4` 和 `slirp-smp4` 场景复测。

### P3：跨架构验证

- 分别跑 `cargo xtask starry app qemu -t ebpf/net_stats --arch <arch>`。
- 检查 `/proc/kallsyms` symbol fragment 是否一致。
- 验证 send/recv byte decode 是否得到合理值。
- 若 ABI 不同，为不同 arch 分支读取寄存器或返回结构。

### P4：真实 net-bench 验证

- 跑 `bash apps/starry/net-bench/run.sh aarch64 slirp`。
- 跑 `bash apps/starry/net-bench/run.sh aarch64 slirp-smp4 --repeat 3`。
- 如环境允许，配置 TAP 后跑 `bash apps/starry/net-bench/run.sh aarch64 tap`。
- 对比 iperf3 指标和 eBPF delta 是否方向一致。

## 可继续使用的命令

eBPF self-test：

```sh
cargo xtask starry app qemu -t ebpf/net_stats --arch x86_64
```

eBPF userspace clippy：

```sh
cargo clippy --manifest-path apps/starry/ebpf/net_stats/net_stats/Cargo.toml --all-targets
```

eBPF fmt：

```sh
cargo fmt --manifest-path apps/starry/ebpf/net_stats/net_stats/Cargo.toml
cargo fmt --manifest-path apps/starry/ebpf/net_stats/net_stats-ebpf/Cargo.toml
```

net-bench SLIRP：

```sh
bash apps/starry/net-bench/run.sh aarch64 slirp
```

net-bench SMP：

```sh
bash apps/starry/net-bench/run.sh aarch64 slirp-smp4 --repeat 3
```

net-bench TAP：

```sh
sudo ip tuntap add dev tap0 mode tap user "$USER"
sudo ip addr add 192.168.100.1/24 dev tap0
sudo ip link set tap0 up
bash apps/starry/net-bench/run.sh aarch64 tap
```

手动汇总：

```sh
python3 apps/starry/net-bench/summarize.py apps/starry/net-bench/results/starry-aarch64-slirp-*.txt
```

## 交接结论

当前已经完成了网络性能测试框架和 eBPF 辅助观测的主要雏形：

- `net-bench` 已具备多场景、多迭代、summary、环境指纹的基础能力。
- `net_stats` 已能在 x86_64 Starry QEMU 中通过自测，TCP/UDP tx/rx byte counter 均有效。
- eBPF 的能力边界、适用场景和准确性限制已写入 `apps/starry/net-bench/EBPF_NET_STATS.md`。

下一步最关键的是把 `net_stats` 从“独立可用的 eBPF app”接入为“net-bench 中真正来自 guest 的 before/after snapshot”，并修正当前 host 侧采样和日志覆盖问题。
