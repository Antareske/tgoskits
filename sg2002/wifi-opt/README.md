# www 内容索引

本目录是 `sg2002/wifi-opt` 分支工作树内的资料区，存放板级镜像、板卡配置、测试数据与分析文档，不被仓库追踪。整理日期：2026-09-24。

同日将工作区 `www/wifi` 下的 SG2002 WiFi 资料并入本目录：`sg2002/wifi-irq` 分支资料成为 `sg2002-wifi-irq/`，2026 年 7 月的重构前分析并入 `before-refactor/`，源目录已删除。

## 目录结构

```
www/
├── images/          板级镜像（约 11 GiB）
├── sg2002/          当前进度（sg2002/wifi-opt）的 SG2002 + AIC8800 资料
│   ├── aic8800/     驱动与吞吐分析文档（9 篇）
│   ├── iperf3-tests/ 双系统 iperf3 对比测试数据
│   ├── wifi-ap/     AP 板卡配置与镜像载荷
│   └── wifi-sta/    STA 板卡配置
├── sg2002-wifi-irq/ 上一分支（sg2002/wifi-irq）资料，2026-08-03 ~ 09-06
└── before-refactor/ 重构前分析、教学与构建记录，2026-07 ~ 09-03
```

## 新旧对照

| 内容 | 时期 | 状态 |
| --- | --- | --- |
| `sg2002/aic8800/`、`sg2002/iperf3-tests/`、`sg2002/wifi-ap/`、`sg2002/wifi-sta/`、`images/` | 2026-09-11 ~ 09-24 | 现行：分析基线与板卡配置对应当前 dev 主线 |
| `sg2002-wifi-irq/newdev/`、`logs/` | 2026-08-28 ~ 09-06 | 上一分支在研记录：所述 aic8800d80 修复已由 PR #2276 合入 dev（`4713bfc98`，2026-09-11），在当前基线中含两份（修复 7 的 CARD_INT 锁死、修复 8d 的 credit 判据）被 dev 上先到的 #2299 / #2305 以不同实现取代；`logs/` 是该轮修复的板测日志 |
| `sg2002-wifi-irq/archive/` | 2026-08-03 ~ 09-03 | 历史：SDHCI 中断、MMIO fence、NAPI 变基等过程资料，代码路径按当时的 `drivers/blk/sdhci-cv1800/` 写法，该目录在当前主线已改为 `cv181x-sdhci/` 与 `sdhci-host/` |
| `sg2002-wifi-irq/sg2002-wifi-fw/` | 2026-09-04（资产来自 01-14 厂商镜像） | 参考：Linux 侧加载链与固件组合调查，对当前驱动的固件/patch 对比仍可用 |
| `before-refactor/`（含 2026-07 分析） | 2026-07 ~ 09-03 | 历史：重构前驱动与旧镜像构建流程；`teaching/aic8800-mechanism/` 讲的是统一 SDIO 重构后的机制，与 `sg2002/aic8800/` 的现行分析同源但更早 |

## images/

| 文件 | 大小 | 内容 |
|---|---|---|
| `2026-01-14-16-03-d4003f.img` | 1.6 GiB | 厂商官方 SD 镜像（LicheeRV Nano）。FAT@扇区 1、ext4@扇区 32769；用于抽取板级 DTB 与 `fip.bin` / `ramdisk.bin` |
| `sg2002_starryos_wifi_ap_iperf3.img` | 2.4 GiB | StarryOS AP 版：内核 dev `59de8ccb3` + `licheerv-nano-sg2002-wifi.toml`，含 AP DTB 与 iperf3 注入 |
| `sg2002_starryos_wifi_sta_iperf3.img` | 2.4 GiB | 在 AP 版镜像上换内核（`c1f5be737`）与 STA DTB，rootfs 不变 |
| `sg2002_starryos_wifi_sta_iperf3_20260917.img` | 2.4 GiB | 在上一版镜像上换内核（`de07a9e38`） |
| `sg2002_starryos_wifi_sta_iperf3_20260924.img` | 2.4 GiB | 在 09-17 版上换内核（dev `9a7b868ba`，编译期凭据 `aasta`），STA 基线轮用 |
| `sg2002_starryos_wifi_camera_actloop_serve.img` | 2.1 GiB | camera actloop serve 版（无 `.json` 构建记录） |

三个带 `.json` 旁注的镜像，旁注记录构建当次的源仓库、提交、板级配置与 DTB；旁注里的路径字段是构建当次的路径，实物当前在本目录。镜像与板卡配置的对应关系见 `sg2002/aic8800/sg2002-ap-investigation.md`。

## sg2002/

### aic8800/（11 篇）

| 文档 | 内容 |
|---|---|
| `aic8800-async-optimization-plan.md` | 数据面异步化（流水线化）执行方案：目标与判据、空口天花板、测量项、阶段 1~4 改动与验证 |
| `aic8800-optimization-tracker.md` | 优化推进的「改动 → 测试 → 现象」周期跟踪（含每轮镜像、分支与判读口径） |
| `probe-round1-20260924.md` | 第一轮探针（阶段 M）测量结果与对方案的影响；原始日志 `probe-round1-20260924.log` |
| `aic8800-driver-principles.md` | 当前驱动的分层原理（驱动核心 / rdif 接入层 / 上层运行时） |
| `aic8800-data-plane-optimization.md` | 数据面异步化分析：TX / RX 逐跳路径、短路条件、分层对比 |
| `aic8800-tx-throughput-analysis.md` | TX 吞吐瓶颈：每帧串行往返的实测证据，取代前一篇的优化优先级 |
| `throughput-bottleneck-analysis.md` | SDIO 吞吐瓶颈静态分析（含板端实测与厂商驱动对照） |
| `throughput-bottleneck-analysis-pub.md` | 上一篇的对外版本（去掉本机路径与内部来源说明） |
| `licheervnano-bus-topology.md` | 板级总线拓扑调查：SDIO / 蓝牙 / USB 走向 |
| `sg2002-ap-investigation.md` | 开机自动 AP 排查记录（含镜像、DTB 与构建命令） |
| `draft.md` | 分层结构草稿 |

### iperf3-tests/

PC（Windows）与板端双系统（vendor Linux / StarryOS）的 iperf3 对比测试。`summary.md` 是结论与工具限制说明；每个 `<os>/<mode>/` 下含 `cases.csv`（24 个用例的参数与结果）、`cases/`（逐用例的两侧日志）、`board_serial.log`（板端串口全程）。`tools/` 是 PC 侧控制脚本（PowerShell）。

### wifi-ap/ 与 wifi-sta/

两块板卡实例的配置与载荷：DTS 源（`lcn.dts` / `lcn-sta.dts`）、编译产出的 DTB、AP 版镜像注入用的 iperf3 载荷（`iperf3`、`libiperf.so.0`、`iperf3-3.19.1-r1.apk`）。

## sg2002-wifi-irq/

上一分支 `sg2002/wifi-irq` 的全量资料，2026-08-03 ~ 09-06。

| 项 | 内容 |
|---|---|
| `newdev/` | 该分支在 dev 基线上做 aic8800d80 修复的追踪（问题清单 → 修复、改动文件说明、PR 描述、交接文档、板测日志整理）；`archive-august/` 是此前一版 PR 时期内容，`sidework/` 是 STA 数据面冻结调查，`wifi-ctl/` 是板端 WiFi 模式控制工具源码 |
| `archive/` | SDHCI 中断方案、MMIO store buffer fence、PLIC、SDHCI 寄存器等教学与分析，以及变基对照与性能回归调查；`9-4-after-wifi-refactor/` 是统一 SDIO 重构之后那一轮的记录（含 rc、评审问题解释） |
| `logs/` | 2026-09-04 轮板测的串口日志（`931.log`、`2.log` ~ `18.log`、`f1.log` 及 PC 侧对照日志） |
| `sg2002-wifi-fw/` | vendor Linux 侧驱动模块与固件资产（`driver-ko/`、`fw/`）、`dmesg-wifi.log`，以及 `linux-fw-investigation.md`（Linux 侧加载链与固件组合调查） |
| `sg2002-image-build/` | 镜像构建技能的副本，与 `~/.claude/skills/sg2002-image-build` 内容一致 |
| `sg2002-image-build-original-20260828` | 指向 `/workspace/.claude/skills/sg2002-image-build` 的失效符号链接（该路径当前不存在） |
| `sg2002-iperf3-inject/` | 镜像注入载荷与 apk（`usr/` 内容与 `before-refactor/sg2002-iperf3-inject/usr/`、`sg2002/wifi-ap/` 重复） |

## before-refactor/

当前工作之前的分析、教学与构建记录：

| 项 | 内容 |
|---|---|
| `sg2002-wifi-performance-analysis.md` | 性能差距根因分析（2026-07-17，基线 TCP 上行 13.7 Mbps 对 vendor Linux 33.2 Mbps） |
| `wifi-analysis.md` | 重构前的驱动实现状况分析（2026-07-24） |
| `wifi-optimization-analysis.md` | 重构前的优化空间与异步优化分析（2026-07-24） |
| `sg2002-starryos-image-build-guide.md` | 早期 StarryOS Wi-Fi SD 卡镜像构建指南（2026-07-25） |
| `sg2002-image-scripts/` | 早期镜像构建与内核/rootfs 快速更新脚本（2026-07-27） |
| `teaching/aic8800-mechanism/` | 统一 SDIO 重构后的 AIC8800 驱动机制教学文档（8 章 + 索引） |
| `sg2002-image-build-20260903-dev-wifi.md` | dev 主线 wifi 镜像构建小结 |
| `sg2002-iperf3-inject/usr/` | 镜像注入载荷（iperf3 及依赖） |

## 注意

- 镜像合计约 11 GiB，只在本机保存；本目录整体不被仓库追踪。
- iperf3 载荷存在三份（`sg2002-wifi-irq/sg2002-iperf3-inject/usr/`、`before-refactor/sg2002-iperf3-inject/usr/`、`sg2002/wifi-ap/`），内容一致（md5 相同）。
- 部分文档中的外部检出路径（其它 clone、BSP 检出）是成文时的机器路径，本次整理未逐一核对。
- `sg2002/iperf3-tests/summary.md` 提到的 `logs/` 目录为测试当次的原始日志，不在本目录。

## 备份说明

本目录是原工作树 `www/` 中文本部分的备份，2026-09-26 收进 www 分支，只含文档与日志。
上文「目录结构」与「images/」两节所述内容中，下列部分未纳入备份，仍只存在于原工作树：
`images/*.img` 板级镜像（同目录的 `.json` 构建旁注已保留）、`sg2002-wifi-irq/sg2002-wifi-fw/`
下的固件与驱动模块（`.bin` / `.ko`）、`sg2002-wifi-irq/sg2002-iperf3-inject/` 与
`before-refactor/sg2002-iperf3-inject/` 的 iperf3 载荷（ELF / `.so` / `.apk`）、
`sg2002/wifi-ap/` 与 `sg2002/wifi-sta/` 的 DTB 与 iperf3 载荷，以及
`sg2002-wifi-irq/sg2002-image-build-original-20260828`（指向不存在路径的失效符号链接）。
