# PR 草稿（未提交）

标题：

```
feat(starry,ebpf): add the netmon network-stack monitor
```

正文：

---

把「网络栈到驱动」的单包延迟做成可复用的 eBPF 监测工具，替代反复手写打点再重编译上板的排查方式。

### 背景与问题

网络性能排查目前只能靠临时 `log::info!` 打点，改一次代码编译一次；而「帧从协议栈提交到总线起始花了多久」「中断到 owner 被调度之间隔了多久」「一次 CMD53 里多少是总线、多少是控制器/中断/准备」这类问题，聚合计数答不出来，需要的是单包延迟的**分布**。更早的一版 kprobe 监测工具挂在重构前的驱动路径上，已随驱动重构失效；它那套无条件轮询与当前 queue runtime 的设计也相冲突。

### 方案

在队列运行时与驱动这条路径上分层挂点，计数与延迟分开处理：

- **挂点**：队列帧边界（`QueueFramePort::transmit/receive`）、调度侧（`PollGroupState::schedule_irq`、`QueueGroupExecutor::poll`）、SDIO 传输（`SdioCard::submit_read_dma/submit_write_dma`）、WiFi 控制面（`AicWifiControl::start`）。每层成对挂入口与返回探针，跨层配对得到供给延迟与唤醒延迟。
- **符号存活**：挂点都是具体非泛型函数并标注 `#[inline(never)]`，否则 release 构建下符号会被内联掉、探针静默失效。
- **解析与降级**：loader 从 `/proc/kallsyms` 解析符号并要求唯一匹配（多重匹配时收敛到值命名空间符号）；镜像不含对应驱动时，SDIO/WiFi 探针按 optional 跳过并告警，只有 required 探针缺席才失败。
- **计时**：共享时间戳槽 + 返回探针读 age，不读被探函数的返回值（绕开 sret ABI 限制）；核内 log2 分桶，避免除法与浮点。
- **输出**：`NETMON_BEGIN`/`NETMON_END` 包裹的可解析行，便于脚本化采样。

### 改动点

| 范围 | 内容 |
| --- | --- |
| `net/ax-net`、`drivers/blk/sdmmc-protocol`、`drivers/net/aic8800` | 6 处挂点注解（`#[inline(never)]`），只保留驱动层与唤醒/供给链相关的点；每接口计数不挂——那些数字 `/proc/net/dev` 已经提供，挂上去只会让记账函数退出内联 |
| `starry-kernel` | kprobe/kretprobe 注册失败返回 `Unsupported` 而不是 panic 内核，`perf_event_open` 路径向上传递；挂点不可插桩是诊断动作的正常结果，不应把内核带走 |
| `apps/starry/ebpf/netmon` | loader（aya）+ eBPF 程序 + 共享常量 + 构建脚本 + QEMU 冒烟配置 |
| `axmm` | 内核地址空间把**可执行区**（内核文本所在区）从块映射改为基页映射 |

`axmm` 这一处是前者的前置修复，必须一起进：kprobe 插桩要改内核文本一页的权限，而改权限走页表层的区域保护路径。若该页处于块映射中，页表层需要先把块拆成基页，而拆分是 break-before-make——先清空整个块描述符、再装入新表；被清掉的这块恰好装着正在执行的代码与陷入向量，于是取指立刻缺页，陷入向量本身也已不可取指，CPU 就在「取指—陷入—向量取指失败」之间死循环，表现为无输出、无 panic 的静默卡死。改为基页后，改权限退化成一次叶子项更新，不再触发拆分。

### 验证情况

- QEMU riscv64 冒烟（`netmon --once`）：退出码 0，全部 required 挂点 attach 成功。
- 跨架构回归（aarch64 QEMU + `ebpf/syscall_count`）：退出码 0。`axmm` 是各架构共享代码，故补此轮确认不影响其它架构启动与 eBPF 功能。
- SG2002 板级构建：目标函数在板级内核中均唯一解析；入口指令按 kprobe 层的 RVC 规则复核可插桩（2 字节对齐本身不是拒绝条件，只有入口编码为 32 位指令且地址非 4 字节对齐才会被拒）。
- **未做**：实板 iperf3 与开/关监测的对比（需要板卡），以及监测扰动量的量化。吞吐类验收数据按设计在监测关闭时采集。

### 测试结果

```
$ cargo xtask starry app qemu -t ebpf/netmon --arch riscv64
[WARN  netmon] resolved sched_irq -> ...
NETMON_BEGIN
irq=0 poll=0 port_tx=0 port_rx=0 sdio_read=0 sdio_write=0 wifi_start=0
...
NETMON_END
（退出码 0）

$ cargo xtask starry app qemu -t ebpf/syscall_count --arch aarch64
SYSCALL_COUNT_PASS: 740 records across 6 syscall ids
（退出码 0）
```

riscv64 冒烟在 attach 完成后立即快照，同一轮没有业务流量，因此计数与直方图均为 0；该轮验证的是挂点是否成功建立，不是数据本身。
