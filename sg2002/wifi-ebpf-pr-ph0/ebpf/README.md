# eBPF 网络栈监测线（本工作树）

本工作树与分支只做一件事：**用 eBPF 监测 StarryOS 网络栈，尤其是栈与驱动的边界**。
aic8800 WiFi 优化在**另一条独立的工作树/分支**上推进（`wt-sg2002-wifi-opt`），
两边互不携带对方的提交。

- 分支：`sg2002/wifi-ebpf`，工作树：`wt-sg2002-wifi-ebpf`（符合 `wt-<分支名>` 约定）
- 当前 tip：`25ca2febd`，尚无 PR
- 基线：`dev` @ `5fd1c6c84`（2026-09-29 变基）。变基前的本分支 tip 备份为
  `backup/sg2002-wifi-ebpf-20260929`，回退用 `git reset --hard <备份分支>`
- 拆分前的工作（含历史 wifi_monitor 那条线）保存在备份分支
  `backup/sg2002-wifi-ebpf-histor-20260924` 与 `backup/sg2002-wifi-opt-20260924-ebpf-split`

## 路线

已从"对网络路径上的 Rust 函数挂 kprobe"转为：

> 内核原生累计计数器作为事实来源，静态 tracepoint 作为可版本化的观测契约，
> 每包上下文随包携带而不在 BPF 侧配对，eBPF 只负责聚合、采样与临时诊断，
> kprobe 降级为开发排障工具。

依据与交叉对比见 `netmon-decision.md`。**阶段零、一、三已完成，阶段二完成栈侧部分**
（均在 QEMU riscv64 上验证，阶段零另有一轮反证对照）。

## 当前进度

阶段零（补 eBPF 子系统的三个安全缺口）：

| 提交 | 内容 |
| --- | --- |
| `a26e37be8` | `axcpu` 新增经异常表的内核地址安全拷贝（四个架构） |
| `727c51a79` | 探针读改为故障安全；加载期拒绝无界程序（含 netmon 分桶改写） |
| `5e2b0082a` | `netmon --selftest`：读、加载、ringbuf 三条自检 + QEMU 判据 |

阶段一（把队列运行时的计数器接出来）：

| 提交 | 内容 |
| --- | --- |
| `15614d1b9` | `NetQueueStats` 带上接口名与 poll group；`/sys/kernel/debug/net_queue` |

阶段二（静态网络 tracepoint，栈侧部分）：

| 提交 | 内容 |
| --- | --- |
| `3d3020dde` | `ax_net::observe` 单入口 + 内核侧 `net:queue_poll` |
| `975d00d71` | `net:queue_irq`（中断序号）与 `queue_poll` 的区间 |
| `1415d26ec` | `net:queue_rearm`、`net:queue_backpressure` |
| `94fe716fc` | 宽事件的参数个数例外 |
| `9a594ccb6` | `net:route_result`（丢包原因 + 接口名）；慢事件明细走 ringbuf 并计其丢失数 |

阶段三（载体与五个区间）：

| 提交 | 内容 |
| --- | --- |
| `cd745fef1` | 区间 1：等待中的中断等了多久 |
| `eb263b80b` | 区间 4/5：载体放在 DMA 令牌上，跨驱动往返 |
| `546d9b9e5` | 区间 2/3：接收帧的投递点与跨 CPU 交接 |
| `53df3e09b` | 采样率做成运行时旋钮；采样与全采样两组可比（旋钮真正可写是 P12 修的，此前的两组对照作废） |
| `a35253044` | 队列与 verifier 测试的编译与预期修正 |
| `25ca2febd` | 发送侧只在采样帧上读时钟，未采样帧的路径上不再有调用 |

kprobe 原型阶段的 5 个提交：

| 提交 | 内容 | 去留 |
| --- | --- | --- |
| `15998d6eb` | kprobe 注册失败返回错误而不是 panic 内核 | 保留（kprobe 仍是排障路径） |
| `1a8bf237b` | 可执行内核区按基页映射（`axmm`） | 保留（与路线无关的正确性修复） |
| `ba70cffb0` | 6 处 `#[inline(never)]` 挂点注解 | 阶段二完成后撤除，撤除本身是对照实验 |
| `05d5cf6e6` | netmon 监测程序（loader + BPF 程序 + 构建 + QEMU 配置） | 保留，改定位为原型与 `netmon debug --kprobe` 通道 |
| `3065b5c1b` | 冒烟判据匹配 CRLF | 保留 |

已打通的：交叉构建（riscv64 musl 静态 loader + 内嵌 BPF 对象）、loader 在真实内核里解析符号、
optional 探针降级、QEMU 运行流程（`cargo xtask starry app qemu -t ebpf/netmon --arch riscv64`）、
kprobe attach 卡点闭环（根因是内核镜像区按块映射建立，改文本权限触发块拆分自毁；
修复为可执行内核区按基页映射）。

**尚未做**：实板上板冒烟（探针开关对比）。镜像与板侧脚本已就绪，操作与判据见
`board-test.md`；负载由 `netmon-board.sh` 的 `NETMON_LOAD` 提供（任何往 stdout 流数据的
下载器即可）。`apps/starry/network-throughput` 面向的是 AKA-00-SG2002，它的板级用例要一个
PC 侧 HTTP 服务提供 session 文件，与本板的取法不是一条路。

## 本目录维护的文档

| 文档 | 内容 |
| --- | --- |
| `netmon-research.md` | 调研：专业 eBPF 网络监测的机制与维度、StarryOS 基础设施现状对照、非侵入性路线对比、可做的创新点、待补基础设施、上板考量 |
| `netmon-decision.md` | 结论与路线：与 `gpt-opion.md` 的交叉对比（一致/补强/修正）、探针契约、eBPF 侧数据结构、落地阶段、验收标准 |
| `netmon-plan.md` | 开发方案：XDP 的边界结论、阶段与交付物、组件边界、数据结构、构建部署、测试验收、风险、**现有提交的去留** |
| `netmon-tracker.md` | 「改动 → 测试 → 现象」周期跟踪（阶段零至四），含归档索引 |
| `board-test.md` | 板测运行手册：镜像怎么出、板侧怎么跑、三组配置与判据 |
| `archive/` | kprobe 原型阶段的历史：方案、跟踪、PR 草稿 |

## 上级 www 里的历史资料

这些是本分支早期工作留下的原始文档，**未被项目追踪**：

| 文档 | 价值 |
| --- | --- |
| `ebpf-lessons.md` | 16 条踩坑与决策（构建、ABI、对齐、静默失效等） |
| `wifi-monitor-test-report.md` + `l1.log` / `l2.log` | 真板实测：下行 100% < 300 µs vs 上行 94.8% 落 20–50 ms；探针开销约 19% |
| `netstacklat-research.md` | 早期方案调研与取舍 |
| `wifi-monitor-*.md` / `wifi-ebpf-evaluation.md` / `rp.md` | 两代旧实现（kprobe 版、raw tracepoint 版）的设计与评审记录，挂点已随驱动重构失效 |
| `bpf-examples/` | Linux `bpf-examples` 的副本（含 `netstacklat`、`pping`） |
