# eBPF 网络栈监测线（本工作树）

本工作树与分支只做一件事：**用 eBPF 监测 StarryOS 网络栈，尤其是驱动层周围**。
aic8800 WiFi 优化在**另一条独立的工作树/分支**上推进（`wt-sg2002-wifi-opt` / `perf/aic8800-tx-credit-accounting`），
两边互不携带对方的提交；本分支从 dev 起，只含下面三个提交。

- 分支：`sg2002/wifi-ebpf`，工作树：本目录（`wt-sg2002-wifi-ebpf`，符合 `wt-<分支名>` 约定）
- 当前 tip：`b45c8b272`
- 拆分前的工作（含历史 wifi_monitor 那条线）保存在备份分支
  `backup/sg2002-wifi-ebpf-histor-20260924` 与 `backup/sg2002-wifi-opt-20260924-ebpf-split`

## 当前进度

| 提交 | 内容 | 状态 |
| --- | --- | --- |
| `323212bd9` | kprobe 注册失败返回错误而不是 panic 内核 | 已完成，内核与 LKM 构建通过 |
| `876c1ecb3` | 6 处 eBPF 挂点注解（`#[inline(never)]`） | 已完成 |
| `b45c8b272` | netmon 监测程序（loader + BPF 程序 + 构建脚本 + QEMU 配置） | 已完成，交叉构建通过 |

已打通的：交叉构建（riscv64 musl 静态 loader + 内嵌 BPF 对象）、loader 在真实内核里解析符号、
optional 探针降级、QEMU 运行流程（`cargo xtask starry app qemu -t ebpf/netmon --arch riscv64`）。

**卡点已闭环**（2026-09-25，详见 `netmon-tracker.md` 周期 E3）：

1. **RVC 入口**：符号地址不落在指令边界上时被探针层判 `InvalidAddress`，相应挂点标 optional；
   本轮 riscv64 QEMU 构建中各挂点均对齐，未触发。
2. **attach 卡住**：根因是内核镜像区在内核地址空间里按**块映射**建立，`patch_kernel_text`
   改文本权限时触发块拆分；拆分的 break-before-make 会先清空整个块描述符，连带解除映射
   正在执行的代码与陷入向量，CPU 随即陷入"取指—陷入—向量不可取指"的死循环。
   修复：可执行内核区改按基页映射（`axmm`），改权限退化为一次叶子项更新。
   QEMU 冒烟中全部挂点 attach 成功，monitor 正常输出 `NETMON_END`。

**路线取舍已定**：走 **B1**（继续用 kprobe，源码侵入最小）；B2（静态 tracepoint）与
B3（内核内直方图）不再需要，作为备选留在 `netmon-tracker.md` 的方案分叉表里。

## 本目录维护的文档

| 文档 | 内容 |
| --- | --- |
| `netmon-plan.md` | 监测方案：目标与判据、hook 分层与配对、阶段 S1~S3、非目标、风险、**构建环境前提** |
| `netmon-tracker.md` | 「改动 → 测试 → 现象」周期跟踪（H1/H2 历史两轮 + E1/E2 本轮）、阶段性收尾、方案分叉 |
| `qemu-smoke-riscv64-20260924.log` | QEMU 冒烟原始日志（attach 结果） |
| `qemu-smoke-kallsyms-20260924.log` | 带 `/proc/kallsyms` 地址的冒烟日志（对齐问题的证据） |

## 上级 www 里的历史资料（本轮工作的来源）

这些是本分支早期工作留下的原始文档，**未被项目追踪**，是本目录方案的依据：

| 文档 | 价值 |
| --- | --- |
| `ebpf-lessons.md` | 16 条踩坑与决策（构建、ABI、对齐、静默失效等） |
| `wifi-monitor-test-report.md` + `l1.log` / `l2.log` | 真板实测：下行 100% < 300 µs vs 上行 94.8% 落 20–50 ms；探针开销约 19% |
| `netstacklat-research.md` | 方案调研与取舍（为何不链式测量等） |
| `wifi-monitor-kprobe.md` / `wifi-ebpf-evaluation.md` / `wifi-monitor-ebpf-intro.md` / `rp.md` | 两代旧实现（kprobe 版、raw tracepoint 版）的设计与评审记录，挂点已随驱动重构失效 |
| `bpf-examples/`、`netstacklat.tar.gz` | 参考资料 |

历史实现（`apps/starry/ebpf/wifi_monitor` 等）在拆分时未带入本分支：它们的挂点依赖重构前的驱动布局，
而 dev 已经整体重构；需要时从上面两个备份分支取回。

## 备份说明

本目录是原工作树 `www/` 中文本部分的备份，2026-09-26 收进 www 分支，只含文档与日志。
下列内容未纳入备份，仍只存在于原工作树：`bpf-examples/`（外部检出的仓库）、
`ebpf/repro/` 的构建产物与 `kallsyms.bin` 复现样本、`netstacklat.tar.gz`
（实际内容是一次失败下载的 404 响应）。
