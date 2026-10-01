# StarryOS eBPF 网络栈监测方案

目标与判据、现状、hook 设计、阶段划分与非目标。历史决策与教训见 `netmon-tracker.md`
与 `archive/`（从历史 worktree 存档的文档）。

## 1. 目标与判据

把「网络栈到驱动」的**单包延迟分布**做成可复用工具，替代反复"手写 `log::info!` 打点 + 重新编译上板"的排查方式。

要回答的三类问题：

| 类别 | 具体问题 | 归属 |
| --- | --- | --- |
| 供给链 | 帧从协议栈提交到总线起始花了多久？是栈没产帧，还是帧在队列里等驱动拉？ | aic8800 优化的 P3 |
| 唤醒 | IRQ 到 owner 被调度之间隔了多久？是否被调度时间片切走？ | aic8800 优化的 A3 |
| 事务内部 | 一次 CMD53 从提交到完成，多少是总线、多少是控制器/中断/准备？ | 阶段 2/3 的定形依据 |

判据：

1. 能给出上述量的**分布**（直方图），不是均值——历史结论「94.8% 落在 20–50 ms」正是分布才看得到的东西；
2. 在 SG2002（riscv64）实板可跑，随镜像交付；
3. 监测扰动**可量化、可关闭**：历史实测 3 个 kprobe 带来约 19% 吞吐下降（rbpf 无 JIT、纯解释），
   因此**吞吐验收测量必须在监测关闭时进行**；
4. 探针数量克制：不需要的功能就删（历史第 15 条教训）。

分工原则（历史付过学费的结论）：**计数用内核埋点 / `/proc`，延迟分布用 eBPF**。
eBPF 对编译器行为（内联决策、mangled 名）有强依赖，不适合作为验收口径的数据源。

## 2. 现状（已核实）

### 2.1 内核侧：已在主线 dev，不需要移植

- `os/StarryOS/kernel/src/ebpf/{mod,map,prog,transform,error}.rs`（约 1100 行），底层 `kbpf-basic` + `rbpf`（**解释执行，无 JIT、无真 verifier**）；
- `os/StarryOS/kernel/src/kprobe.rs`（约 630 行）+ `perf/{kprobe,tracepoint,raw_tracepoint,uprobe,bpf}.rs`：kprobe/kretprobe、cooked/raw tracepoint、uprobe、perf buffer；
- `bpf(2)` 已接（`syscall/mod.rs`），`/proc/kallsyms` 已提供（`pseudofs/proc.rs`）；
- helper：`bpf_probe_read`、`bpf_probe_read_kernel`(113→4)、`bpf_get_current_pid_tgid`(14)、`bpf_get_current_comm`(16)、`bpf_ktime_get_ns`(5，唯一时间源)；
- map：per-CPU Array/Hash、ringbuf（已实现但无应用使用）。

**缺**：fentry/fexit（无 BTF）、skb/tc/XDP 挂点、`bpf_spin_lock`、`bpf_get_smp_processor_id`、`bpf_probe_read_user`(112)；
`register_kprobe` / `register_kretprobe` 失败时 **`.expect()` 直接 panic 内核**（`kprobe.rs`）——历史上
RISC-V 的 AUIPC+JALR 序言触发 `UnsupportedInstruction` 时炸过一次，必须修（见 §4 S1）。

### 2.2 应用侧：netmon 已写好但从未上板

`apps/starry/ebpf/netmon/`（未合入分支 `feat/net-enhance`，提交 `3ece4e98c`）：
15 个 BPF 程序 + 455 行 loader + 共享常量。**它是按当前驱动架构写的**（挂
`SdioCard::submit_*_dma`、`AicWifiControl::start`），比更早的 wifi_monitor 线可移植。
但从设计文档自己的记录看，只做过**静态符号验证**（x86_64 内核 ELF 中符号存在且唯一），
**板级验证待做**。

### 2.3 可丢弃的历史分支与其数据

更早的 `sg2002/wifi-ebpf`（kprobe 版）与 `sg2002-wifi-ebpf-raw-tracepoint`（tracepoint 版）
挂在**重构前**路径（`components/aic8800/...`、`components/sdhci-cv1800/...`、`wifi_glue.rs`），
在 #1951、#2201 之后已不可用；dev 的设计文档也显式否决了那一版的「全局回调 + 轮询 + 10 ms kicker」。
它们不可移植，但留下了两组可用的东西：

- **实测数据**：下行 `SDIO_WR_LAT` 100% < 300 µs；上行 94.8% 落在 20–50 ms 桶（当时上行仅 82.5 Kbps）；
  探针开销 19%（12 → 9.77 Mbps）。
- **方法**：8 桶固定阈值直方图、"同一探针两种时机对比"的判读方式。

## 3. 方案

### 3.1 hook 分层与配对

| 层 | 挂点 | 采什么 | 配对 |
| --- | --- | --- | --- |
| ~~L3~~ | ~~`ax_net::router::DeviceHandle::count_tx/count_rx`~~ | **不挂**：/proc/net/dev 已提供，挂它只会让记账函数退出内联 | — |
| L2 | `QueueFramePort::transmit/receive`（+ kretprobe） | 帧计数 + 采样帧时长 | ↔ L0 得供给延迟 |
| L1 | `PollGroupState::schedule_irq`、`QueueGroupExecutor::poll`（+ kretprobe） | IRQ 计数、**IRQ→poll 唤醒延迟**、poll 时长 | 内部配对 |
| L0 | `SdioCard::submit_read_dma/submit_write_dma`（+ kretprobe） | CMD53 计数 + 时长分布 | ↔ 核心 emit 得适配层开销 |
| 控制面 | `AicWifiControl::start`（+ kretprobe） | 控制命令计数 + 时长 | — |
| 新增 1 | `OwnerOutputs::take_tx_frame` ↔ L2 提交时刻 | **帧在 rdif 队列的逗留时间** | 区分「栈没产帧」/「帧在等拉」 |
| 新增 2 | `AicOwner::advance_with_cause` 出入 | owner 步数与每步耗时 | — |
| 新增 3 | 核心 `emit` / `consume_transmit_data` | 核心视角 RTT、准备耗时 | ↔ L0 |
| 新增 4 | `drivers/blk/sdhci-host` command submit / 完成 IRQ | CMD53 内部再拆分 | 内部配对 |

采样：计数类全量，每包计时类按 1/4 采样（`SAMPLE_MASK = 3`）。

### 3.2 数据通道

- 计数：per-CPU Array，核内只自增，用户态跨 CPU 求和；
- 分布：核内 log2 分桶（32 桶，`leading_zeros` 实现，无循环）写 per-CPU Array，
  用户态 dump（避免核内除法与浮点）；
- 跨 hook 时间戳：共享非 per-CPU 的 `Array<u64>` 单槽，读侧取 age 后清零，
  并设陈旧窗口过滤跨 CPU 配对（`IRQ_POLL_MAX_NS = 1<<27`）与同 CPU 异常（`EVENT_MAX_NS = 1<<31`）；
- 时基：`bpf_ktime_get_ns`（唯一时间源）；**不读被探函数的返回值**（绕开 sret ABI 限制）；
- 输出：可解析行（`NETMON_BEGIN/END`）+ 单次/间隔采样，不走串口逐行打印。

### 3.3 部署

静态 musl 二进制，用镜像构建的 `--inject` 直接注入 rootfs（不搬历史分支的 overlay/prebuild 机制）。

### 3.4 明确不做

- `apps/starry/net-bench/` 那套板测基建；
- `qemu-*.toml` 里的 CI 断言与 `--test/--once` 自测分支；
- socket 层监测（无 skb/BTF，纯 eBPF 不可行）与 raw tracepoint 版（对源码侵入过大，已被放弃）；
- **待定**：`/proc/net/queue`（内核 +72 行）虽属 proc 集成，但其导出量
  （`irq_to_poll_remote_wake`、`budget_exhaustion`、`missed`、`rearm_race`）正是唤醒/调度侧证据，
  建议与监测一起评估后再决定去留。

### 3.5 构建环境前提（务必成对升级）

| 组件 | 要求 | 原因 |
| --- | --- | --- |
| eBPF 侧工具链 | 默认 `nightly`（LLVM 23） | aya-build 用它编译 BPF 程序 |
| `bpf-linker` | 0.11.x（内嵌 LLVM 21/22/23） | 必须能读上面那版 nightly 产出的位码；0.9.x（LLVM 19）会报 `Unknown attribute kind` |
| LLVM dev 包 | `llvm-23-dev` | 仅当需要从源码编 bpf-linker 时才要；实际用的是官方预编译 musl 静态产物 |
| aya | 钉在 `5c1a79e0`（0.14.0 的一个 rev） | 该 rev 走 `perf_event_open` + `PERF_EVENT_IOC_SET_BPF` 挂 kprobe；更新的 rev 会尝试 `BPF_BTF_LOAD`/`BPF_LINK_CREATE`，内核尚未实现 |
| 交叉工具链 | `/opt/riscv64-linux-musl-cross` | loader 编成静态 musl 二进制 |

注意：项目根的 `.cargo/config.toml` 用了 `include = [...]` 这个新配置键，LLVM 19 时代的 cargo 无法解析 ——
**不能靠"降级工具链"来迁就旧 bpf-linker**，只能把 bpf-linker 升上去。

## 4. 阶段

| 阶段 | 内容 | 完成判据 |
| --- | --- | --- |
| S1 | 前置修复（kprobe 注册失败不 panic）✅ + 挂点注解（6 处，已按范围收敛）✅ + 搬 netmon ✅ | 实板冒烟：各层计数非零、直方图有分布；关监测后吞吐回到基线 —— **待内核 attach 卡点修好** |
| S2 | 新增 §3.1 的四个点 | 产出「帧在 rdif 队列逗留时间」「owner 步耗时」「CMD53 内部拆分」三类分布，回答 aic8800 侧的 P3 |
| S3 | 按需深化：跨层配对严格化、SDHCI 中断侧拆分、必要时 Phase 2（config-gated tracepoint） | 由 S2 的数据决定是否立项 |

## 5. 验证

- 实板：SG2002 + aic8800D80 + STA 镜像（与本仓库 aic8800 优化线同一套镜像流程）；
- 每阶段的冒烟口径：**计数非零**（历史上曾因 `AYA_BPF_TARGET_ARCH` 未设而静默全零）、
  直方图有分布、无内核 panic；
- 扰动：同一轮 iperf3 分别在开/关监测下跑，记录差值；
- 与 aic8800 优化线的关系：**优化的验收数字必须在监测关闭时采集**。

## 6. 风险

| # | 风险 | 缓解 |
| --- | --- | --- |
| R1 | 编译器行为变化（内联、mangled 名）导致探针**静默失效** | 唯一符号断言；每轮上板校验计数非零；挂点注解进代码 |
| R2 | kallsyms 片段随 rustc 版本漂移 | 片段集中定义，失败即报错而非取第一个匹配 |
| R3 | 监测扰动（历史 19%/3 探针） | 采样策略；探针数量克制；验收测量分离 |
| R4 | 单槽时间戳被并发调用方覆写 | 先确认调用方数量；必要时 per-thread key |
| R5 | 目标函数被深内联、偏移不可知（历史 RX 字节失败） | 只挂函数边界，不做字段偏移读取 |
| R6 | 内核中不支持序言的函数 panic | S1 的前置修复 + 挑"大函数"做挂点 |

## 附录 A：挂点注解清单（dev 现状为零）

| 文件 | 处数 |
| --- | --- |
| `drivers/blk/sdmmc-protocol/src/sdio/io/transfer.rs` | 2 |
| `drivers/net/aic8800/src/rdif/device/endpoints/control.rs` | 1 |
| `net/ax-net/src/queue_runtime/executor/mod.rs` | 3 |
| `net/ax-net/src/queue_runtime/state.rs` | 1 |
| `net/ax-net/src/router.rs` | 2 |

## 附录 B：历史文档索引

历史 worktree（`../ebpf/wt-*`，均为个人 fork、未合入）中与本题相关的文档，
已存档到 `archive/`：

| 文档 | 价值 |
| --- | --- |
| `ebpf-lessons.md` | 16 条踩坑与决策（最高复用价值） |
| `wifi-monitor-test-report.md` + `l1.log` / `l2.log` | 94.8% vs <300 µs 的原始证据与静默全零症状 |
| `sg2002-netperf-ebpf-design-20260901.md` | netmon 全栈设计（hook 表、约束、阶段划分） |
| `netstacklat-research.md` | 方案调研与取舍（含为何不链式测量） |
