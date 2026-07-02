# Starry 网络优化：QEMU 性能 + 基线测试落地方案（WSL2 / Windows PC）

> 目标：在**没有专用物理机机房**的条件下，用 **QEMU + WSL2 + 一台 Windows PC** 这套
> 多数开发者手边就有的环境，尽量把 Starry 网络栈的吞吐 / 延迟 / PPS / CPU 效率 / 多核扩展
> 测得**可量化、可复现、可解释**，并跑出**同口径的 Linux 基线**用于对照。
>
> 本文档基于最新 dev 分支状态（commit 6a857920）更新，反映 poll 模型重构后的
> 测试环境搭建要求和观测重点调整。
>
> 配套文档：
> - `starry-net-performance-analysis.md` —— 瓶颈定位（测什么是为了验证哪条结论）。
> - `starry-net-benchmark-methodology.md` —— 通用度量方法论（指标体系、报告结构）。
>
> 本篇专讲：**在 WSL2/Windows 这套受限环境里，怎么把 QEMU 测准。**

---

## 0. 决策：为什么主用 QEMU，物理机降级为对照基线

`benchmark-methodology.md §0` 把测试拓扑按推荐度排序为：物理机 > QEMU+TAP+vhost-net >
QEMU+TAP（无 vhost）> SLIRP（不可用于性能测试）。物理机噪声最低、最接近 Linux 基线，
但代价高：环境一致性、网卡/驱动能力匹配、Starry 侧可观测性自建、专业打流硬件，门槛逐一叠加。

本项目落在 WSL2 / Windows PC 上，因此调整为：

| 角色 | 拓扑 | 用途 | 状态 |
|------|------|------|------|
| **主力** | QEMU + TAP + vhost-net（WSL2 内） | 全套指标、优化前后对比、多核扩展 | **执行** |
| **降级对照** | QEMU + TAP（无 vhost） | vhost 不可用时的功能/趋势兜底 | 备选 |
| **禁用** | QEMU `-netdev user`（SLIRP，当前默认） | 仅功能冒烟，**不产出任何性能结论** | 禁用于压测 |
| **未来展望** | 物理机 + 真实网卡 | 终极上限校准 | **本项目范围外，不承担** |

> 仓库现状：`scripts/self-compile.sh:279`、`scripts/axbuild/src/rootfs/qemu.rs:63` 默认
> `-netdev user`（SLIRP）。压测前必须切换拓扑，详见 §2。

物理机 + 真实网卡是噪声最低、最接近 Linux 真实差距的拓扑，但其环境一致性、网卡/驱动能力匹配、
专业打流硬件等门槛较高。**本项目不承担物理机测试工作**，仅作为未来展望保留（见 §7），所有
日常 KPI 与优化前后对比均以 QEMU+vhost 数据为准。

---

## 1. 环境分层：先认清 WSL2/Windows 会引入哪些噪声

要测准，先承认这套环境的**结构性噪声源**，并逐一压制：

```
Windows 11 PC
 └─ Hyper-V / WSL2 轻量虚拟机（Linux 内核, 即 host）
     ├─ vCPU 由 Windows 调度器分配 ← 噪声源 A：宿主调度抖动
     ├─ 动态内存 / vmmem 回收      ← 噪声源 B：内存压力与回收停顿
     ├─ TAP/bridge 在 WSL2 内核     ← 噪声源 C：虚拟网卡转发开销
     └─ QEMU 进程（被测 guest）
         ├─ KVM 加速（需 WSL2 嵌套虚拟化）← 噪声源 D：无 KVM 则 TCG，数据无意义
         ├─ vCPU 绑定                ← 噪声源 E：vCPU 在物理核间迁移
         └─ Starry / Linux guest
```

| 噪声源 | 后果 | 压制手段（见 §3 纪律） |
|--------|------|------------------------|
| A 宿主调度抖动 | 吞吐方差大、尾延迟飘 | `.wslconfig` 固定 `processors`；Windows 侧设高性能电源计划、关闭后台 |
| B 内存回收 | 周期性停顿、P999 尖刺 | `.wslconfig` 固定 `memory`、关 `pageReporting`、给足 swap |
| C TAP 转发 | 吞吐被虚拟网卡限制 | 用 vhost-net 卸载；收发同机走同一 bridge，减少跨设备 |
| D 无 KVM | 退化为纯 TCG 解释执行，慢几十倍且不可比 | **必须确认 `/dev/kvm` 存在**；否则方案不成立 |
| E vCPU 迁移 | cycles/packet 抖动 | QEMU `taskset` 绑核 + `-smp` 固定；guest 内绑核 |

> **硬性前置检查（不通过则全部数据作废）：**
> 1. `ls /dev/kvm` 存在且可用（WSL2 需 Win11 + 嵌套虚拟化开启）。
> 2. `qemu-system-* --accel help` 含 `kvm`，启动参数用 `-accel kvm -cpu host`。
> 3. `.wslconfig` 已固定 CPU/内存，且测试期间不跑其他重负载。

---

## 2. 测试拓扑搭建（QEMU + TAP + vhost-net，全在 WSL2 内）

核心思路：**收发两端都在同一台 WSL2 内**，用 Linux bridge + 两个 TAP 把
“被测 guest” 与 “Linux 对端 guest / WSL2 host 进程” 连起来，避免跨越 Windows 网络栈
（Windows vEthernet/Hyper-V switch 会引入不可控 NAT 与抖动）。

### 2.1 推荐拓扑 A：guest ↔ WSL2 host（最简，单被测 VM）

```
[Starry/Linux guest in QEMU] --virtio-net(vhost)--> tap0 --+
                                                            br0 (WSL2 内 bridge)
[WSL2 host 上的 iperf3/netperf 服务端] <--------------------+
```

- 被测放 guest，打流/对端工具（iperf3 server、netperf netserver）跑在 WSL2 host。
- 优点：只需一个 QEMU 实例，最省资源；vhost-net 把虚拟网卡数据面下沉到 WSL2 内核。
- 缺点：host 侧是“理想对端”，不能体现两端都是 Starry 的场景（本项目对端固定 Linux，符合
  `methodology §2.1` “对端固定一台 Linux” 的纪律）。

### 2.2 推荐拓扑 B：guest ↔ guest（双 VM，更贴近真实对打）

```
[被测 guest] --tap0--+         +--tap1-- [Linux 对端 guest]
                     br0 (WSL2 bridge)
```

- 两个 QEMU 实例挂同一 bridge，一端被测、一端 Linux 对端。
- 更接近 `methodology §4.1` 实验矩阵中“被测 vs 固定 Linux 对端”的隔离。
- 资源占用翻倍，绑核要更小心（见 §3）。

### 2.3 QEMU 网络参数（关键：vhost + 多队列 + offload 显式可控）

在 `qemu.rs:DEFAULT_ROOTFS_WIRING`（`scripts/axbuild/src/rootfs/qemu.rs:23-28`）当前是
`virtio-net-pci,netdev=net0` + `user,id=net0`。压测拓扑需替换为：

```
# WSL2 host：建 bridge + tap（参考 scripts/net/qemu-tap-ifup.sh 思路，README 已提及）
sudo ip link add br0 type bridge
sudo ip addr add 10.0.2.1/24 dev br0
sudo ip link set br0 up
sudo ip tuntap add dev tap0 mode tap
sudo ip link set tap0 master br0
sudo ip link set tap0 up

# QEMU：vhost-net + TAP（替换默认的 -netdev user）
-device virtio-net-pci,netdev=net0,mq=on,vectors=10,\
        csum=on,gso=on,host_tso4=on,host_tso6=on,guest_tso4=on,guest_tso6=on \
-netdev tap,id=net0,ifname=tap0,script=no,downscript=no,vhost=on,queues=4
```

- `vhost=on`：数据面卸载到 WSL2 内核 vhost-net 线程，绕开 QEMU 用户态，**这是 QEMU 拓扑接近
  真实吞吐的关键**。
- `mq=on,queues=4` + `vectors=10`：开多队列，为验证 `analysis §3.1` 的多队列/RSS 改造预留通道。
  注意 Starry 侧当前是单队列（`e1000/mod.rs:19` `QUEUE_ID0`），多队列要等驱动侧改造后才有效，
  先用单队列建立 baseline。
- offload 开关（`csum/gso/tso`）显式列出：用于验证 `analysis §2.4` 校验和/TSO offload 打通
  前后的差异——**先全关跑 baseline，再逐项打开对比**。

> 落地建议：不要改动 `qemu.rs` 默认值（它服务于功能 boot/CI），而是新增一个**压测专用
> profile**（独立 toml 或脚本注入这组参数），避免污染常规 CI 路径。

---

## 3. 降噪纪律：让 WSL2 上的数字尽量稳定可比

这是本方案的核心。WSL2/Windows 比物理机噪声大，必须靠纪律把方差压到可接受。

### 3.1 Windows 宿主层

- 电源计划设“高性能/卓越性能”，关闭睡眠与 CPU 降频（C-state/Turbo 波动会污染 cycles/packet）。
- 测试期间关闭杀毒实时扫描、Windows Update、Defender 对 WSL 目录的扫描、其他 VM。
- 关闭 Windows 的“内存压缩”对 vmmem 的干扰（实测期间不要让物理内存吃紧）。

### 3.2 `.wslconfig`（固定资源，消除动态分配抖动）

```ini
[wsl2]
processors=8          # 固定 vCPU，>= QEMU 要用的核数 + host 工具核数
memory=16GB           # 固定内存，避免动态回收停顿
swap=0                # 关 swap，避免换页尖刺；或给足且固定
pageReporting=false   # 关闭内存回收上报，减少周期性停顿
nestedVirtualization=true   # 必须：保证 /dev/kvm 可用
kernelCommandLine=isolcpus=4-7   # 可选：隔离部分核专供 QEMU
```

### 3.3 QEMU / guest 绑核

- QEMU 用 `taskset -c 4-7` 绑到隔离核；`-smp` 与绑核数一致。
- 拓扑 B 双 VM：被测与对端绑到**不同物理核**，打流器与被测**不抢核**。
- guest 内用 `taskset`/CPU affinity 把 iperf3、netperf 绑到固定核，避免 guest 内迁移。
- vhost-net 内核线程（`vhost-$pid`）也建议绑到独立核，避免和 vCPU 抢。

### 3.4 测量纪律（对齐 `methodology §4.2`）

- 每个数据点 **≥ 5 次**，取均值 + 标准差；**标准差 > 10% 的数据点判为不可信，需排查噪声后重测**。
- 每次测试前 `warmup`（丢弃前 1–2 次结果），让 cache/JIT/路由表预热。
- 固定测试时长（如 iperf3 `-t 30`），固定 MTU（先 1500），固定 CPU 频率。
- 记录每次的环境指纹：WSL 内核版本、QEMU 版本、`.wslconfig`、绑核映射、offload 开关状态、
  Starry commit / baseline tag。

> **WSL2 特有的可比性红线：** 由于宿主是 Windows 调度，**Starry 与 Linux 基线必须在
> 完全相同的 `.wslconfig`、相同绑核、相同 QEMU 参数、相同 host 负载下背靠背连续测**，
> 中间不重启 Windows、不改电源计划。否则两组数字不可比。

---

## 4. 指标与工具（沿用 methodology §1/§2，标注 QEMU 适配）

六维指标不变（吞吐 / PPS / 延迟 / 连接速率 / CPU 效率 / 多核扩展），工具选型针对本环境：

| 维度 | 工具 | QEMU/WSL2 适配要点 |
|------|------|--------------------|
| 吞吐 | iperf3 `-P`/`-R`/`--bidir` | 同二进制 guest 与对端都跑；vhost 开启后才有意义 |
| PPS/小包 | netperf UDP_RR、iperf3 `-u -l 64` | 小包最吃每包开销，最能暴露 `analysis §2.1/2.2` 拷贝与加锁 |
| 延迟 | netperf TCP_RR/UDP_RR、sockperf | WSL2 调度抖动会抬高 P999，需 §3 绑核后才可信 |
| 连接速率 | netperf TCP_CRR、nginx+wrk | 压 `analysis §3.2` listen_table/端口分配锁 |
| CPU 效率 | **Starry：架构周期计数器埋点**；Linux 基线：`perf stat` | cycles/packet 是核心 KPI，见 §5 |
| 多核扩展 | 全套 × smp1/smp4 × 绑核 | 复用 `test-suit/starryos/qemu-smp1`、`qemu-smp4`（`methodology §2.3`） |

### 4.1 仓库内可直接复用的资产

- **`os/arceos/tools/bwbench_client`**：raw-frame 裸帧带宽基准，绕过 TCP/UDP，隔离
  **驱动+设备层 vs 协议栈层**瓶颈。配合 §2 的 TAP，`sudo ./target/release/bwbench_client
  [sender|receiver] tap0`（README:19-27），guest 侧需配套 raw-frame app/test-suit case。
- **`apps/qperf`**：基于 `tgoskit-harness_kit` 的 QEMU plugin + analyzer，产出 **flamegraph**。
  这是 Starry 侧“慢在哪”的主分析产物，对应 Linux 侧 `perf record` 火焰图。
- **`test-suit/starryos/qemu-smp1` / `qemu-smp4`**：现成单核/4 核框架，多核扩展曲线直接复用。

---

## 5. 可观测性：Starry 没有 eBPF，自建“穷人版”（对齐 methodology §3）

QEMU 环境下，Linux 侧观测最强但只能用于**基线机**；Starry 侧必须自建埋点。

### 5.1 Linux 基线侧（在 WSL2 host 或 Linux 对端 guest 上）

- `perf stat -e cycles,instructions,cache-misses,LLC-load-misses`：算 cycles/byte、IPC。
  **注意：WSL2 的 PMU 可能不暴露硬件 cache 事件**，需确认内核 `perf` 是否支持；不支持时
  cycles/instructions 仍可用，cache-misses 可能为 0/不可用，须在报告中标注。
- `bpftrace`/`bcc`：`net:netif_receive_skb`、`skb:kfree_skb`（drop reason）、`tcp:tcp_probe`。
  **WSL2 内核需带 BPF + tracepoint 支持**（自编译 WSL 内核或用发行版内核时确认 `CONFIG_BPF`）。
- 火焰图：`perf record` → 与 Starry qperf 火焰图并排，圈出多出的拷贝/锁/校验和热点。

> eBPF/perf 只跑在 Linux 上。Starry 自身没有 BPF runtime（`methodology §3 注`）。

### 5.2 Starry 侧自建计数（与 `analysis §4` 统计建议一致）

- per-socket / per-device 原子计数：rx/tx packets+bytes、drop（按原因：环满 / ARP pending
  满 / 无路由 / 畸形包）、retrans、queue-full、poll 次数。等价于 Linux `kfree_skb drop reason`。
- 拷贝次数计数：在 `analysis §2.1` 拷贝点（`to_vec`、各 `copy_from_slice`）加可开关计数。
- 阶段 cycles 采样：在 `poll_interfaces`、`Interface.poll`、设备 recv/send 前后取周期差。
- 锁竞争：用仓库 **`components/lockdep`** 观测 `SERVICE`/`SOCKET_SET` 全局锁持有/等待，
  验证 `analysis §3.1` 锁拆分前后差异。
- qperf 火焰图汇总上述热点。

---

## 6. 执行流程与报告（对齐 methodology §4/§5）

### 6.1 实验矩阵（每个里程碑全跑）

```
拓扑   = QEMU+TAP+vhost（主）；偶尔 QEMU+TAP 无 vhost（兜底）
对端   = 固定 Linux（WSL2 host 或对端 guest），记录内核/QEMU/网卡型号(virtio)/MTU
被测   ∈ {Linux 基线(同 QEMU), Starry baseline tag, Starry 本次改动}
核数   ∈ {1, 2, 4}     ← smp1/smp4 复用；绑核固定
负载   ∈ {TCP 单流, TCP 16流, UDP 64B PPS, TCP_RR, TCP_CRR, nginx+wrk}
offload∈ {全关(baseline), 逐项开(csum/tso)}   ← 验证 analysis §2.4
MTU    = 1500（jumbo 待实现后单测）
```

> 关键：**“Linux 基线”也跑在同一 QEMU+vhost 拓扑里**（同 virtio-net、同 vhost、同绑核），
> 这样 Starry vs Linux 的差距才剔除了虚拟化层，反映的是协议栈本身。物理机基线（§7）是
> 另一条独立校准线。

### 6.2 每格记录

吞吐 Gbps（均值+标准差，≥5 次）、PPS、延迟 P50/P99/P999、cycles/byte、cycles/packet、
整体 CPU%、丢包/重传（按原因）、多核扩展比。

### 6.3 呈现

- 三条对比线：Linux(同 QEMU) 基线 / Starry 优化前 / Starry 优化后，标注“达到 Linux 的百分比”。
- 多核扩展曲线：吞吐 vs 核数（1/2/4），三方同图。
- 火焰图并排：Linux perf vs Starry qperf。
- 归因表：每项指标变化对应到哪个改动（去拷贝 / 锁拆分 / offload / 批处理）。
- **环境指纹附录**：`.wslconfig`、绑核映射、QEMU/内核版本、offload 状态、commit/tag。

### 6.4 落地清单（最小可用 → 完整）

**第一步：搭 QEMU 基线环境（1–2 周）**
1. 确认 `/dev/kvm` + 嵌套虚拟化；写定 `.wslconfig`（§3.2）。
2. 搭 br0 + tap0/tap1，QEMU 切到 vhost+TAP profile（§2.3），**废弃 SLIRP 压测**。
3. WSL2 上跑通 iperf3/netperf/nginx+wrk + perf stat（+ 尽力 bcc），确立 Linux(同 QEMU) 基线。
4. Starry 上跑通同一组工具（同二进制），得到 baseline tag 数字。

**第二步：建可观测（与优化并行）**
5. Starry 加 per-socket/device 计数 + 拷贝计数 + 阶段 cycles（`analysis 阶段一`）。
6. 接 qperf 火焰图；lockdep 观测全局锁竞争。
7. 用 bwbench_client 隔离驱动层 vs 协议栈层。

**第三步：每次优化固定动作**
8. 按 §6.1 矩阵全跑，产出三方对比 + 多核曲线 + 火焰图 + 归因表 + 环境指纹。
9. 把多核扩展比与 cycles/packet 作为**项目级 KPI 主指标**逐里程碑跟踪。

---

## 7. 未来展望：物理机对照基线（本项目范围外）

物理机 + 真实网卡是噪声最低、最能反映“距 Linux 真实差距”的拓扑，可作为 QEMU 数据可信区间的
上限校准。但其门槛较高（环境一致性、网卡/驱动能力匹配、专业打流硬件、跨两端同型号同 MTU 同绑核
等 `methodology §0` 纪律），**不在本项目承担范围内**，此处仅作未来展望记录。

若未来有人接手这条校准线，可参考方向：与 §6.1 相同的负载矩阵、固定 Linux 对端、网卡型号对应
`drivers/net`（e1000/ixgbe/realtek）、单独成列与 QEMU 数据并排，标注两套拓扑的系统性偏差。

> 本方案的日常 KPI 与优化前后对比**全部以 §6 的 QEMU+vhost 流程为准**，不依赖物理机数据。

---

## 8. 已知风险与边界（WSL2/QEMU 环境诚实声明）

- **方差天花板**：即便做满 §3 降噪，WSL2 因宿主调度仍可能比物理机方差大。尾延迟（P999）
  结论需特别谨慎，建议以 P50/P99 为主，P999 仅作趋势参考。
- **vhost 依赖**：若 WSL2 内核未编 `vhost_net`，退化到无 vhost TAP，吞吐绝对值会低且方差大，
  此时**只看 Starry 优化前后的相对趋势**，不与 Linux 绝对值对比。
- **PMU 限制**：WSL2 可能不暴露完整硬件 PMU，cache-miss 类指标可能不可用，报告需标注缺失项。
- **多队列受限**：Starry 单队列，QEMU `queues=4` 在驱动改造前不产生扩展收益，先建单队列 baseline。
- **绝对值非权威**：QEMU 绝对吞吐不代表真实硬件上限（物理机校准为未来展望，见 §7，本项目不承担）；
  本方案的价值在于**优化前后可比、Starry vs Linux 同拓扑可比、多核扩展趋势可信**。
