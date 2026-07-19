# 实习周报（2026-07-18）

## 本周工作进展

本周工作围绕 net-bench 网络性能测试套件的统计维度完善展开。上周已建立了端到端的吞吐基线，但能回答的只是"跑多快"，缺少 CPU 效率、协议开销拆解和异常归因这三个分析维度。本周在应用态监测、Linux 基线补全和内核态 `/proc` 接口三个方向补上了这些缺口，累计 17 个提交。

### 一、应用态监测：net-bench 测试框架集成

上周的 net-bench 能回答"吞吐是多少"，但无法回答"CPU 效率如何"和"协议开销占比多大"。本周在测试框架中集成了 perf stat 和协议开销分析，同时补全了 Linux 基线以建立公平的跨内核对比基准。

[#1417](https://github.com/rcore-os/tgoskits/pull/1417)

#### 1. CPU 效率统计（perf stat）

单纯比较吞吐量忽略了 CPU 资源的投入差异——比如 StarryOS 可能以 100% CPU 占用跑出 1 Gbps，而 Linux 仅用 20% CPU 就跑出同样吞吐，这个差距在吞吐数字上完全不可见。因此需要引入 CPU 效率指标来补充吞吐维度的盲区。

实现上，在 `run.sh` 中新增 `--with-perf` 选项，在 QEMU 外侧通过 `perf stat` 采集四个核心指标：`cycles`、`instructions`、`cache-references`、`cache-misses`。perf 输出与 iperf3 结果统一进入 `results/` 目录，由 `summarize.py` 在一次调用中完成网络吞吐和 CPU 效率的联合汇总，自动计算并渲染：

- **IPC**（Instructions Per Cycle）：`instructions / cycles`，衡量 CPU 执行效率，越高说明 CPU 越少"空转"
- **cache-miss-rate**：`cache-misses / cache-references`，反映内存访问模式对 cache 的友好程度，过高通常意味着数据局部性问题

旧的独立脚本 `run-with-perf.sh` 标记为废弃——保留以支持额外的 `LLC-load-misses` 计数器，但新开发统一走 `run.sh --with-perf` 入口。

#### 2. 测试入口与 Linux 基线

net-bench 的测试入口经过两周迭代，本周稳定为双入口设计：

**StarryOS 侧**由 `run.sh` 承担，参数显式指定、不做隐式环境探测，确保每次运行的可复现性。支持的参数矩阵覆盖 `--scenario`（5 种拓扑）、`--arch`（x86_64/aarch64）、`--accel`（kvm/tcg）和 `--repeat`（跨启动方差），以及前述 `--with-perf` 和 `--no-summary`。

**Linux 基线侧**由 `run-linux-baseline.sh` 承担。核心思路是在与 Starry 完全相同的 QEMU+vhost 拓扑下运行 Linux guest，使性能差距直接归因到内核栈本身。本周补全了 x86_64 支持：按架构自动生成 QEMU 命令、宿主机无内核时自动通过 apt 拉取 Ubuntu generic 内核（virtio built-in）、guest 内 iperf3 负载与 Starry 侧 `net-bench-common.sh` 同步。网络设备参数对齐 Starry vhost TOML 的裸 `virtio-net-pci + tap,vhost=on`，保证拓扑严格可比。initramfs 通过 busybox + iperf3 + libcrypto 最小化打包——完整 Alpine rootfs（>1.5G）解包会触发 write error 导致 guest panic。

公共流程抽象至 `core/lib.sh`（`nb_*` 函数族），统一了 iperf3 服务端生命周期管理（进程级 trap，确保中途中断也不残留孤儿进程）、参数校验、环境指纹记录和结果汇总。宿主机配置由 `bin/setup` 一键完成（br0/tap0 网桥、dnsmasq DHCP、vhost_net 模块加载、设备权限），`bin/teardown` 通过 `.bench-state.json` 状态文件实现精确回滚。

配置矩阵方面，20 个 QEMU TOML 文件覆盖 5 场景 × 2 架构 × 2 加速器的全组合（`apps/starry/net-bench/qemu/`，命名规范 `{scenario}-{arch}-{accel}.toml`）。SLIRP 仅用于功能冒烟，vhost-net 为主力性能拓扑，vhost-smp4/tap-smp4 覆盖多核扩展。axbuild 重构后 x86_64 需通过 UEFI pflash 启动（不再使用 `-kernel` 直接加载），本周修复了全部 10 个 x86_64 配置的 `uefi=false→true`。

#### 3. 协议开销分析

网络吞吐数字包含各层协议头的开销，而应用真正关心的是有效载荷吞吐。在 `summarize.py` 中新增 Protocol Overhead 段，对比 `/proc/net/dev` 的 L2 字节总数与 iperf3 应用层字节总数，计算 TX/RX 方向上的协议开销比。

以太网帧头（14B）+ IP 头（20B）+ TCP 头（20B）= 每数据包 54B 的固定开销。这个开销的占比取决于包大小：对于 128KB 的大包，协议开销仅约 0.04%，基本可忽略；但对于 64B UDP 小包，协议开销可达 45.8%，意味着近一半的带宽被协议头吃掉。统计时排除 warmup 迭代，仅对测量迭代计算，避免冷启动效应污染结果。

### 二、内核态监测：/proc/net 接口补全

上周的 eBPF net_stats 方案在技术验证中暴露了两类根本性问题：RX 方向因 `RxToken::consume` 被重度内联导致参数偏移无法确定，以及 kretprobe 的 sret ABI 问题（返回值在 RAX 而非 RDI）。这些问题本质上源于 eBPF 对编译器行为的强依赖——在内核快速迭代阶段，任何内联决策或寄存器分配的变化都可能导致探针静默失效。因此本周转向内核埋点方案，直接在协议栈关键路径放置 `AtomicU64` 计数器，从根本上规避编译器不确定性。

#### 1. /proc/net/dev L2 帧统计

[#1571](https://github.com/rcore-os/tgoskits/pull/1571)

核心实现在 `ax_net::router` 的 `DeviceHandle` 中增加四个 `AtomicU64` 计数器：`rx_bytes`、`rx_packets`、`tx_bytes`、`tx_packets`。计数点的选择遵循"覆盖完整数据路径"的原则：

- **RX 路径**：`count_rx(net_payload_len)` 在设备 RX worker 接收到帧后调用，计入 L2 帧总长
- **TX 路径**：`count_tx(net_payload_len)` 在 dispatch 路径帧发出前调用
- **回环接口**：仅在注入成功后计数，保证统计一致性

对外暴露上，实现 `render_proc_net_dev()`（`os/StarryOS/kernel/src/pseudofs/proc.rs`），输出对齐 Linux 的 16 列格式，guest 内任何进程均可通过 `/proc/net/dev` 读取。相比 eBPF 方案，内核埋点不受编译器内联影响、不被协程多路径干扰，且无需额外加载工具即可通过标准接口访问，更适合集成到 CI 流程中。

#### 2. 错误/丢包维度

[#1645](https://github.com/rcore-os/tgoskits/pull/1645)

仅有字节和包计数不够——吞吐下降时，需要知道是"发不出去"（errors）还是"主动丢弃"（dropped），两者的优化方向完全不同。在 L2 计数器基础上新增 `rx_errors`、`rx_dropped`、`tx_errors`、`tx_dropped` 四个维度，输出对齐 Linux `dev_seq_printf_stats()` 的 17 列格式。

各计数器的触发路径覆盖了网络栈的关键决策点：

- **rx_errors**：ARP 畸形包、Ethernet 设备 `send_to` 返回错误、驱动层 deferred RX error 队列排出
- **rx_dropped**：Router RX 队列满（`RX_QUEUE_SIZE=256` 耗尽）、buffer 分配失败、loopback 注入到 RX 队列失败
- **tx_errors**：路由查找无匹配（含 loopback 回退路径）、`request_arp` 全部重试失败
- **tx_dropped**：设备 TX 队列满、MTU 超限拒绝

同时修复了几个细节问题：ARP 层原先的冗余 `tx_drops` 被移除并统一归类为 `tx_errors`（ARP 请求失败本质是发送错误而非主动丢包）；回环接口的计数从无条件改为仅成功后计数，失败路径计入 `rx_dropped`。新增 4 个边界测试覆盖畸形包、缓冲区满、发送失败等场景，累计 68 个测试通过。

此外，`/proc/net/snmp` 骨架已搭建（Tcp/Raw/Udp 四协议段结构），待 smoltcp 层逐协议累计计数器到位后即可填充数据，为后续 TCP 重传率、UDP 丢包率等协议栈健康度指标提供基础。

#### 3. eBPF 延迟监测可行性分析

虽然 eBPF 不再作为基础统计数据来源，但在延迟细粒度 trace 场景下仍有独特价值——`/proc/net/dev` 只能给出聚合计数，无法提供单包级别的延迟分布。对网络栈的代码路径做了分析：

- **RX 方向可行**：`RxToken::consume` 内部通过 smoltcp 回调完成全部入站协议处理（IP 解析→协议解复用→TCP 状态机→socket 投递），在 phy_rx 处设 kprobe 入口记时 + kretprobe 出口算 delta，即可覆盖完整入站延迟
- **TX 方向 consume 无意义**：`TxToken::consume` 仅为 buffer 分配+内存写入，真正的 TX 处理链（路由查找→ARP 解析→设备分发→驱动发送）在 `Router::dispatch()` 和 `device_tx_worker` 异步任务中，需要不同的插桩点

在 SG2002 WiFi 驱动优化场景中，eBPF 的定位是作为驱动层辅助诊断工具（SDHCI 等待延迟、DMA 完成中断响应时间、TX/RX 描述符环利用率等），是对 `/proc/net` 聚合统计的有效补充。

### 三、SG2002 WiFi 吞吐差距分析参考

邵志航的 SG2002 WiFi 上行吞吐排查报告将 StarryOS WiFi 上行从 ~0.2 Mbps 提升至 ~13.7 Mbps（达到 Linux 33.2 Mbps 的 41%），但仍有约 2 倍差距。剩余差距的根因拆解为三个层次：

| 因素 | 影响占比 | 说明 |
|------|---------|------|
| HE (802.11ax) vs HT (802.11n) PHY 速率差 | ~60% | StarryOS 当前仅协商 HT，Linux 可协商 HE；MCS 索引和空间流数差异导致物理层速率基础不同 |
| SDHCI PIO vs DMA 软件开销 | ~30% | PIO 模式下每次寄存器读写需 CPU 轮询等待，引入约 50ms 调度惩罚 |
| 聚合深度差异 | ~10% | A-MSDU/A-MPDU 帧聚合策略差异影响 MAC 层效率 |

值得注意的是，最大的两个因素（PHY 速率和 PIO/DMA）都属于硬件交互层面而非协议栈算法层面。SDHCI PIO 引发的 50ms 调度惩罚远超协议栈处理本身的延迟——**即便极端情况下重复全拷贝也不会达到毫秒级**。这意味着 StarryOS 与 Linux 的网络吞吐差距应以流水线优化为重点方向（DMA 替代 PIO、多帧拼包减少中断、TX/RX 并行处理），而非单纯的协议栈算法改进。该结论对后续 SG2002 实机测试的优化优先级排序有直接指导意义。

### 四、文档更新

本周文档更新伴随代码改动同步进行，覆盖三个层面：

**eBPF net_stats 系列**（4 个提交）：记录了从 eBPF 方案探索到最终搁置的完整技术决策过程——kretprobe sret ABI 限制的根因（RAX vs RDI，跨 x86_64/aarch64 的寄存器差异）、编译器内联对探针偏移量的影响、以及符号过度匹配（loader 匹配 19 个符号）的过滤策略。这些文档为后续 eBPF 在延迟探测等辅助场景中的使用保留了技术上下文。

**net-bench 文档**（1 个提交）：随测试入口统一和配置矩阵扩充，修正了过时的配置描述、场景说明和结果分析章节，确保文档与当前双入口、20 配置矩阵、vhost-net 主力拓扑等实现一致。

**代码内文档**：在 `ax-net` 的 router、ethernet、device 模块以及 StarryOS 的 proc 伪文件系统中，为新增的计数器和渲染函数补充了触发路径和列格式语义的注释。

### 五、其他

对齐 Nginx 和 Apache CI 配置。

[#1649](https://github.com/rcore-os/tgoskits/pull/1649)　[#1650](https://github.com/rcore-os/tgoskits/pull/1650)

## 下周工作计划

### 一、SG2002（LicheeRV Nano）真实硬件 WiFi 测试

SG2002 小车明日到货。基于本周的吞吐差距分析结论，硬件到位后的首要目标是建立 StarryOS 真实硬件的吞吐基线并定位流水线瓶颈（PIO→DMA 收益评估为最高优先级），同时利用 eBPF 对驱动层做辅助性能测量。

具体工作：

- **硬件上电与基础连接**：上电验证、串口连接（板载 UART 转 USB）、WiFi STA 模式连接 AP
- **WiFi 驱动验证与基准建立**：验证 AIC8800DC 驱动基本功能（扫描、关联、DHCP），iperf3 TCP/UDP 吞吐基线，对比 Linux 同环境数据
- **eBPF 驱动性能测量**：定位 SDHCI PIO 等待延迟、中断响应时间、TX/RX 描述符环利用率等驱动层热点
- **HE（802.11ax）协商排查**：基于已有 HT 对齐修复，继续排查 HE 管理帧交互和能力信息元素协商

### 二、net-bench 应用层统计闭环

当前 Protocol Overhead 只能做 L2→L7 两点对比，中间 L3/L4 层开销不可见。下周在 socket 层补充 L7 有效载荷统计后，在 `summarize.py` 中完成 L2→L3→L4→L7 的逐层开销拆解。同时补充 packets 维度在 benchmark 报告中的利用率分析（包大小分布 vs 吞吐效率），为后续 MTU 和 TCP segment 策略调优提供数据。

### 三、/proc/net/dev 统计完善

当前 `tx_errors` 与 `tx_dropped` 的区分存在语义模糊：`Device::send()` 返回的 `usize=0` 同时涵盖错误、丢包和延迟发送三种语义（`virtio-net` 返回 0 仅表示描述符环满需重试，并非真正丢包）。下周需细化 Device trait 返回值语义，使 errors 与 dropped 精确区分。同时评估 `/proc/net/snmp` 的数据填充可行性——骨架已搭建，需评估 smoltcp 现有 socket 统计结构的可利用程度。

### 四、网络栈 eBPF 延迟探针

基于本周的可行性分析，在 RX 路径 `RxToken::consume` 上实现 kprobe 延迟探针原型：入口记录时间戳（BPF map），kretprobe 出口计算 delta 并输出 log2 直方图，得到 P50/P95/P99 延迟分布供 `summarize.py` 消费——聚合数据回答"丢了多少"，延迟分布回答"卡了多久"。

## 总结

本周在 net-bench 的统计维度上补上了三个关键缺口：CPU 效率分析让吞吐有了效率参照系，协议开销分析让各层开销可量化，`/proc/net/dev` 的错误/丢包维度让异常有了归因路径。eBPF 的角色也从基础统计工具重新定位为辅助诊断手段，在延迟 trace 和驱动性能测量场景中保留。下周 SG2002 硬件到位后，将正式启动真实硬件上的性能基线建立和流水线瓶颈定位工作。
