# StarryOS 网络栈 eBPF 监测开发方案

`netmon-decision.md` 回答"为什么走这条路"（结论、交叉对比、验收口径），
`netmon-research.md` 是调研依据。本文件回答"具体怎么做"：阶段划分、每个阶段改哪些文件、
加什么类型、判据是什么。

进度记录见 `netmon-tracker.md`；kprobe 原型阶段的方案与跟踪已归档到 `archive/`。

## 一、目标与非目标

**目标**

1. 把"网络栈到驱动边界"的**延迟分布与跨层归属**做成可复用工具，替代反复手改打点、
   重编译、上板的排查方式。
2. 观测面以**契约**形式暴露：定长、无指针、版本化、可关联、可统计自身丢失率。

**非目标**

- 不做可编程数据面（XDP 形态的丢包/改包/重定向），理由见 §二。
- 不复制 Linux 的 XDP / tc / skb / cgroup 模型。
- 不重复 `/proc/net/dev` 已有的累计统计。
- 本期不动驱动内部实现，只在**驱动边界**上观测。
- 不改动任何被项目追踪的 `.gitignore`，不动与任务无关的文档。
- **不覆盖 SMP 与跨架构**：目标平台 SG2002 是 riscv64 单核，所以观测结论只针对这个形态。
  多核下的 per-CPU 语义、以及 aarch64 那份拷贝例程，都不在本项目的验证范围内
  （aarch64 的 `kernel_copy.S` 因此只有编译保证）。
- **不覆盖 `/dev/net/tun`**：TUN/TAP 端口是软件队列（`TapPort` 直接实现 `EthernetFramePort`），
  不经队列运行时，因此没有轮询周期、中断、DMA 这些被观测的对象；五个区间在那里无从定义。
  路由层的 `net:route_result` 仍然覆盖它们。

### 设计原则

本方案应当是 StarryOS 式的，但不是刻意求异：Linux eBPF 生态里相当一部分机制
是在补偿它自身的前提（内核与工具分离构建、版本漂移、C 结构体没有类型契约）。
StarryOS 不具备这些前提，合理的实现就是把补偿省掉，用类型系统与所有权结构直接表达。

| # | 原则 | 依据与做法 |
| --- | --- | --- |
| 1 | 类型即契约 | 一次构建同时产出事件与 loader，不存在布局漂移，因此不需要 CO-RE/BTF。payload 是 Rust 结构体，布局由编译器保证；`format` 与 schema 保留给 filter 引擎与用户态工具，但不再是唯一真相 |
| 2 | 事件按自己的模型命名 | 运行时结构是 poll group、generation、budget、owner_cpu，不是 `napi`/`skb`/`qdisc`。**问题从 Linux 实践取，措辞从自己的结构取**；IRQ 与 poll 不是一对一，就按 sequence 区间建模 |
| 3 | 测量在所有者处完成 | 栈是自己的，`QueueGroupExecutor::poll` 自己知道起止时刻，没有理由把两个时间戳交给 BPF 相减。BPF 只做 O(1) 聚合——这也解释了为什么解释执行下这条是硬要求 |
| 4 | 观测边界 = 所有权边界 | `rdif-eth` 是驱动交出帧的统一边界，对四个驱动一致。Linux 的 XDP 要每驱动各写一遍，因为它的驱动是独立代码单元；这里的驱动无关观测点是结构自带的 |
| 5 | 上下文随包携带 | 包的所有者是栈自己，有地方放这个字段。实现落在 **`DmaBuffer`**——唯一走完"协议 → 队列 → 驱动 → 回来"全程的对象；`PacketMeta` 覆盖栈内各层。不在观测系统里重建关联状态 |
| 6 | 观测设施可完全移除 | observer 默认空操作、事件可按 `enable` 关闭、探针可标 optional。监测不成为被监测对象的依赖，与 `no_std` 优先、能力用 trait 边界表达的习惯一致 |
| 7 | 不假装拥有没有的东西 | 无 BTF 就不引入 fentry/fexit；无 verifier 就不把裸包指针交给 BPF（因此 XDP 的本体不做）；无 JIT 就把指令数当成本指标；无 skb 就自建载体 |

**边界：形状对齐 Linux，实现顺着 StarryOS。** 判断某个机制该复用还是该省掉，
标准是"它在补偿 Linux 的前提，还是解决一个我们也有的问题"：

| 机制 | 处置 | 原因 |
| --- | --- | --- |
| `ax-tracepoint` 框架 | 直接复用 | `enable`/`filter`/`trace_pipe`/事件 id 解决的是运行时可开关、用户态可控、可条件过滤，我们同样需要 |
| static key / jump label | 不省（待实测后决定） | 热路径每包过一次，原子读加分支乘起来不可忽略——物理约束相同，不是照抄 Linux |
| CO-RE / BTF | 省掉 | 补偿版本漂移，我们没有这个问题 |
| `format` 文件 | 保留但降级 | 作为 filter 引擎与用户态工具的输入，不是权威 |
| tracepoint 命名风格（`系统:事件`）与 tracefs 路径 | 保留 | 用户态工具（aya）直接依赖这个形状，属接口对齐而非补偿 |

## 二、关于 XDP 的边界（明确结论）

**挂点位置存在且自然，但以 XDP 的形态实现它既不安全也不必要。**

XDP 在 Linux 的语义位置是"驱动收到 DMA 缓冲、栈还没消费"那一刻。StarryOS 有完全对应的位置：
`IRxQueue::reclaim()` 返回 `RxCompletion { buffer: DmaBuffer, packet_len }`，
调用点在 `net/ax-net/src/queue_runtime/executor/mod.rs:676`（`QueueGroupExecutor::poll` 内）。
因为它在 `rdif-eth` 边界上而不是驱动内部，天然对所有驱动
（aic8800 / e1000 / rtl8125 / fxmac）一致——这一点比 Linux 的 per-driver XDP 更好。

但 XDP 的本体能力需要一堆前置条件，StarryOS 目前一条都不具备：

| XDP 的本体能力 | 前置条件 | StarryOS 现状 |
| --- | --- | --- |
| 直接包访问（读或改） | verifier 做边界检查 | 无 verifier；且 `register_allowed_memory(0..u64::MAX)` 还关掉了 rbpf 自身的检查 |
| 丢包 / 改包 | 同上，外加长度调整与 headroom 管理 | 无 |
| `XDP_TX` / `XDP_REDIRECT` | `DEVMAP` / `CPUMAP` / `XSKMAP` + `bpf_redirect*` | 前三种 map 返回 `EPERM`，重定向 helper 不存在 |
| 开销可控 | 指令预算 | 无（一个死循环会卡死 RX 路径） |

所以分成两件事：

- **观测**不需要 XDP。在同一个位置发一个 tracepoint（payload 带 `packet_len`、queue id、
  载体），BPF 拿不到写指针，安全且信息量足够。
- **可编程数据面**才是 XDP 的本体，它是一条独立且大得多的工程线，前置是 verifier
  （或至少一个带边界检查的有界"直接包访问"子集）、重定向 map 与 helper、指令预算。
  **本期不做，也不作为本方案的依赖。**

## 三、阶段与交付物

### 阶段零：补安全缺口（eBPF 子系统，非网络专属）

目标：让"探针本身"不再是故障源。这是后续所有阶段的前置。

三条的共同背景：当前 eBPF 子系统是解释执行、无有效 verifier、无运行期限额，
因此**加载期没有兜底，只能在运行期兜**。

#### Z1 故障安全的读

**现状**：`bpf_probe_read`（helper 4）在 kbpf-basic 里的实现是
`dst.copy_from_slice(src); Ok(())`——一个普通 memcpy，**永远返回成功**。
地址没映射就在内核里触发页错误；地址映射了但偏移错了就静默读到垃圾。

**关键澄清**：`os/StarryOS/kernel/src/perf/nofault.rs` **不能用于此**。它是 AArch64 专用
（依赖 `El1` 页表与 `ax_cpu::mmu::read_kernel_page_table()`，后者只有 aarch64/x86_64 有定义），
且只读单个对齐的 u64 字，用途是 PMU 硬中断栈回溯。SG2002 是 riscv64，这条路不通。

**可行机制**：`components/axcpu/src/exception_table.rs` 的 `fixup_nofault_exception()`，
四个架构的 trap handler 都已接入。缺的是**内核地址方向、任意长度、可在 IRQ 上下文使用**的封装
——`os/StarryOS/kernel/src/mm/access.rs` 那套是用户内存专用，且
`access_user_memory()` 断言 IRQ 使能并显式拒绝 IRQ 上下文，探针恰好就在该上下文里。

**做法**：
1. 基于异常表实现 `copy_from_kernel_nofault(dst, src, len) -> Result<(), Fault>`；
2. 在 `init_ebpf()`（`os/StarryOS/kernel/src/ebpf/mod.rs`）里**覆盖** helper 表项 4
   （kbpf-basic 是 crates.io 依赖，不改它；现在补 14/16、别名 113 就在这同一处），
   让 113 的别名自然跟随；
3. 覆盖时保留原有签名与返回约定（成功 0，失败返回负错误码），使现有程序语义不变。

**验证**：由 `netmon --selftest` 驱动：一个 cooked tracepoint 程序对
`SELFTEST_UNMAPPED_ADDR`（地址空间最后一页，高于所有架构上的全部映射）做一次读、
对一个已知取值的映射地址做一次读，结果分别记入 `fault` / `bad_ok` / `good_ok` 三个计数器；
触发用进程自己的下一次 `openat`（`syscalls:sys_enter_openat` 是 `ax-tracepoint` 既有事件）。

**判据**：`fault ≥ 1`、`bad_ok = 0`、`good_ok ≥ 1`（后者要求读回的值与写入的标记完全相等，
因此"读成功但内容是垃圾"同样判失败）。

#### Z2 有界执行（加载期判定）

**现状**：rbpf 0.4 的解释器没有指令计数器。它的执行入口
（`interpreter::execute_program`）是 crate 私有自由函数，内部是一个无钩子的循环，
**外部无法插入计数**；`register_allowed_memory(0..u64::MAX)` 还关掉了 rbpf 自身的边界检查
（源码带 TODO/FIXME）。

**后果**：一个不终止的循环会在**探针触发的那个上下文**里永久自旋，包括 IRQ 路径。
板子直接卡死，而且这个失败模式极难查——E3 那轮已经吃过一次：内核卡住后日志不再输出，
加的 10 处打点一个都没打印，导致两轮定位走错方向（真因是日志为非阻塞入队，串口 worker
被饿死后记录被丢弃）。

**方法调整**：运行期预算这条路在 rbpf 上走不通，而且它并不划算——100 万条指令的上限
在 IRQ 上下文里同样是数十毫秒的失控。改为**在加载期证明有界**：只接受控制流不会重访
指令的程序，于是 n 条指令最多执行 n 条。这比运行期预算更强，且代价在加载期，
探针路径上为零。

**做法**：`os/StarryOS/kernel/src/ebpf/verify.rs` 对预处理后的指令流做一次线性扫描，
拒绝两类编码：

- **回边**（分支目标 ≤ 当前指令下标）：循环的行程数无法在不跟踪寄存器值的前提下界定；
- **非 helper 的调用**（`BPF_PSEUDO_CALL` 与尾调用）：子程序调用的返回边落在调用点之后，
  同一段子程序被多个调用点到达时，单遍论证不成立。实测当前 aya 产出的程序里没有伪调用
  （kprobe 段 27 次、kretprobe 段 24 次调用全部 source=0），这条限制今天不付出代价。

校验点选在 `load_prog`（`os/StarryOS/kernel/src/ebpf/prog.rs`），即 `BPF_PROG_LOAD` 的唯一
入口——与 Linux 一致：程序在加载期被拒，而不是在运行期被中止。

**连带改动**：现有 netmon 的 `bucket_of` 是一个 while 循环，LLVM 保留了它的回边
（实测 kprobe 段 1 处、kretprobe 段 6 处），改写为展开的位归约（6 次比较，无回边）。

**验证**：`selftest_unbounded` 是一个 `while` 循环形式的程序；`netmon --selftest`
**只调用 `load()`，不 attach**，因此内核若接受它，记录的是计数器而不是真的跑起来。
**判据**：加载被拒且内核存活；`steps = 0`。

#### Z3 ringbuf 可用性

**现状**：`BPF_MAP_TYPE_RINGBUF` 在 kbpf 里有实现（2 个 meta 页 + 数据页，内核虚拟地址双重映射），
但**树内没有任何 app 用过**；其 mmap 页布局是 kbpf 自己的方案
（页 0 = `RingBuf` 结构体 + `consumer_pos`，页 1 = `producer_pos`/`data_pos`，数据从页 2 起），
**不是 libbpf 的布局**，与 aya 侧消费者是否兼容未验证。

**性质**：这不是安全问题，是"有没有这个工具"的问题。方案里 ringbuf 只承担慢事件与错误事件明细，
不在关键路径上。

**做法**：最小端到端——一个 BPF 程序 `bpf_ringbuf_reserve` + `submit`，
用户态 mmap 后读出一条记录。参照 `apps/starry/ebpf/` 下现有 app 的形态。

**判据**：读出正确记录即"可用"；读不出则**明确记为"本方案不使用 ringbuf"**，
慢事件明细也走 map 轮询。两种结果都能落地，需要避免的是到阶段三才发现。

#### 阶段零的验证环境

三条都**只在 QEMU 验证，不上板**，理由见 §七。其中 Z1/Z2 属于"故意让内核失控"的测试，
在板上做需要断电且可能损坏 rootfs；QEMU 上卡死直接杀掉即可。

### 阶段一：把已有数据接出来

目标：`NetQueueStats` 的 12 个字段今天每事件都在算却拿不到，先给它一个出口。

| 任务 | 落点 |
| --- | --- |
| 队列统计的公开出口 | `net/ax-net`：与 `net_dev_stats()` 同级新增队列统计快照 API |
| 队列身份 | `NetworkQueueRuntime` 按组记录 `(接口名, poll group id)`，与 `group_states` 同序同源 |
| `/sys/kernel/debug/net_queue` | `os/StarryOS/kernel/src/pseudofs/debug.rs` 的 debugfs 根增加 `net_queue`。不放在 `/proc/net`：那个目录的内容是与 Linux 对齐的兼容面，而这两样都是 Linux 没有命名的诊断面（理由见跟踪文档的『落点的修正』）|
| 速率计算 | 用户态 loader 负责，内核不维护"每秒值" |

`NetQueueStats` 目前只有计数器，没有身份：多队列下无法分辨哪一行是哪条队列，
因此快照一并带上接口名与 poll group id（与 `NetDevStats` 带 `name` 同理）。

格式取"表头 + 空格分隔行"，与 `/proc/net/dev` 同形，便于 `split_whitespace` 解析：

```
iface queue owner irq schedule missed poll_batches budget_exhaustion spurious probe_deferred rearm_race last_irq_cpu last_poll_cpu irq_to_poll_remote_wake
```

尚未产生事件的 CPU 列写 `-`（`last_irq_cpu`/`last_poll_cpu` 是 `Option`）。

**判据**：QEMU 下数值随流量变化；与 `/proc/net/dev` 口径不重叠（前者是队列/调度侧，
后者是接口累计）。流量用 QEMU 用户态网络自带的宿主网关：`qemu-*.toml` 增加
`[host_http_server]`，guest 侧从 `10.0.2.2:<port>` 下载一次（`qemu-e1000` 用例已是此法），
再读 `/sys/kernel/debug/net_queue`。

### 阶段二：静态网络 tracepoint（低频与边界优先）

目标：先上开销可忽略、信息量最大的事件。

| 事件 | 发射点 | 说明 | 状态 |
| --- | --- | --- | --- |
| `net:route_result` | `net/ax-net/src/router.rs` 的发送错误与超 MTU 路径 | reason 用 `RouteDropReason` 有限枚举，记录里带接口名 | 已接通；访客到不了：超 MTU 的条件是 IP 包长 > 1500，IPv4 分片使它不成立，`Stopped` 一类要走执行器组已消失的路径 |
| `net:queue_backpressure` | `net/ax-net/src/queue_runtime/executor/mod.rs` 的 token 耗尽路径 | reason 用 `QueueStallReason`（设备忙 / 链路断 / 其他） | 已接通；纯下载填不满 TX ring，需要故障注入或实板 |
| `net:queue_rearm` | 同上（`rearm_race` 计数处） | | 已验 |
| `net:queue_irq` | `net/ax-net/src/queue_runtime/state.rs::schedule_irq` | 带 `sequence`，供 poll 侧按区间统计 | 已验 |
| `net:queue_poll` | `net/ax-net/src/queue_runtime/executor/mod.rs::poll` | 带 `duration`、`work`、`blocked` | 已验 |
| `net:queue_poll_irq` | 同上 | 带 `irqs_absorbed` 与等待中的中断时长。与 `queue_poll` 分开是两个问题分开：一轮花了多少、覆盖了多少；tracepoint 宏按字段数生成函数，九字段一条记录过不了参数个数的 lint | 已验 |
| `wifi:control` | `drivers/net/aic8800` 的控制入口 | | **不做**：WiFi 层正在别处重构，形状会变；本分支的交付面是 StarryOS 网络栈 |
| `wifi:sdio_xfer` | `drivers/blk/sdmmc-protocol` 的 CMD53 提交/完成 | | **不做**：同上 |

阶段三另加了四个逐包事件：`net:frame_enqueue`、`net:frame_dequeue`（接收侧）与
`net:driver_submit`、`net:driver_complete`（发送侧），均已验。

**载荷偏移的契约**：eBPF 侧按固定字节偏移读 sample buffer，偏移错了只会读到垃圾而不会报错，
这是风险 R2 的落点。两侧不共享 crate（内核不能依赖 app 侧的 crate，app 侧的 eBPF 程序也不宜
依赖 `ax-net`），改为**由 loader 在 attach 前解析 `/sys/kernel/debug/tracing/events/net/<event>/format`
并核对偏移表**：这份文本由 `define_event_trace!` 在同一处生成，是框架自己的契约，
核对不通过就拒绝 attach——满足"不允许静默产出全零指标"。

判据：QEMU（riscv64）下 cooked attach 成功、计数随流量非零、无内核告警。

**实现中确认的三件事**（都是踩过才知道的）：

1. **分发由 tracefs 的 `enable` 决定，不由 attach 决定。** 内核对 cooked tracepoint 的门控是
   `callbacks_enabled`（由 `events/<sys>/<event>/enable` 写入置位），而 perf 侧只负责把
   `perf_enabled` 置位。两者都开才会发事件——所以 loader 在 attach 之后必须写一次 `enable`。
2. **`format` 的字段行没有分号**：`field: u64 duration_ns offset: 24; size: 8; signed: 0;`，
   只有四个 common 字段写成 `field: u16 common_type; offset: 0;`。解析要按 `offset:` 定位，
   不能按 `;` 分段。
3. **两个 wifi 事件在 QEMU 验不了**（镜像里没有 aic8800/SDIO 驱动）。栈侧的五个事件里
   `queue_poll`/`queue_irq`/`queue_rearm` 在 QEMU 下有非零计数，`queue_backpressure` 与
   `route_result` 只做到"接通且偏移核对通过"——纯下载负载填不满 TX ring，也造不出超 MTU 丢弃；
   `wifi:control` 与 `wifi:sdio_xfer` 的**定义与发射点**可以写，但计数只能在板上取。

### 阶段三：载体与跨层关联

目标：回答"慢在哪里"，而不只是"某个函数执行了多久"。

| 任务 | 落点 |
| --- | --- |
| 载体字段 | **`DmaBuffer`**（`drivers/interface/rdif-eth/src/lib.rs`），即那个走完全程的 DMA 令牌 |
| 逐包事件 | `net:frame_enqueue` / `net:frame_dequeue`（接收侧）、`net:driver_submit` / `net:driver_complete`（发送侧） |
| 采样 | 每 64 帧填充一个载体；速率运行时可调（`/sys/kernel/debug/net_sample_rate`，0 = 不采样） |
| 区间 1 | `PollGroupState` 记"本轮最早一次中断的时刻"，轮询入口取走并算差值 |

**载体位置的改动（与方案原文不同）**：方案写的是放在 `TxSubmitOptions` 与 `RxCompletion`。
实际放在 **`DmaBuffer`** 上，因为它是唯一一个已经走完"协议 → 队列 → 驱动 → 回来"全程的对象：

- 驱动只是**移动**这个令牌，不认识这个字段，所以**四个驱动一行都不用改**（方案原文的 R3
  "波及所有驱动"因此不成立）；改 `ITxQueue::reclaim` 的签名才会真的波及所有驱动。
- 发送完成时拿回来的是**属于这一帧的那个**载体，不需要假设"完成按提交顺序返回"。
- 接收侧同样用它：帧被投递时打戳，协议侧取走时算差值。

代价是每个令牌多 8 字节（池是启动时一次性预分配的，所以是常数开销）。

**采样率取 1/64 而非 1/1024**：实测这个负载只有约 2300 个 TX 帧，1/1024 采不到样本；
1/64 让一段短负载也能出直方图。真实链路上这个值应该更大，所以它被做成了运行时旋钮
（`/sys/kernel/debug/net_sample_rate`），三组对照正是为这个取舍提供依据。

要量出的五个区间：

```
IRQ → queue poll
queue poll → RX publish
RX publish → protocol consume
protocol TX → queue submit
queue submit → DMA completion
```

判据：五个区间各自给出 log2 分布；同一负载下 probe-off / on-unsampled / on-sampled 三组可比。

### 阶段四：按需 flow / socket 诊断

有限容量 LRU flow 表，支持 TCP RTT、重传、建连与 socket 阻塞时间。当前是单 smoltcp
protocol owner，首要回答三个问题：protocol executor 是否饱和、socket workload 是否导致长 poll、
queue owner 是否被 backpressure 拖慢。

**落地前的三点核对（已做）**：

1. **LRU map 有现成的**：内核 eBPF 侧实现了 `BPF_MAP_TYPE_LRU_HASH`
   （kbpf-basic 0.6 的 `map/lru.rs`，基于 `lru` crate），所以"有限容量 flow 表"直接用内核 map，
   不必在 eBPF 程序里自己写淘汰。
2. **"是否饱和"有现成的落点**：`net/ax-net/src/poll_runtime.rs` 的 `ProtocolPollBudget`
   （连续 10 次轮询 / 2 ms 为界，注释写明"既不允许待处理的 socket 也不允许到期的软定时器
   无限占用 CPU"）与 `net/ax-net/src/service.rs::poll`。在这条边界上加一个
   `net:proto_poll` 静态事件（时长、是否做了事、rx 是否有待处理、预算是否用尽），
   与队列侧的 `queue_poll` 同形 —— 这是"protocol executor 是否饱和"的直接答案。
3. **RTT 与重传目前取不到**：`tcp.rs::tcp_info_snapshot` 是 ax-net 唯一从 smoltcp 取 socket
   状态的地方，但它**没有 RTT 字段**，`retransmits` 也留在默认值 0（未填）；smoltcp 的
   `TcpSocket` 通过 ax-net 用到的那组 API 也不暴露丢包重传计数。
   取到它们的两条路都不合适：按固定偏移读 smoltcp 内部结构，违背本方案"偏移必须与框架自身
   契约核对"的原则，且 smoltcp 的结构不是我们的 ABI；改 smoltcp 依赖则超出本分支范围。

**据此落地的口径**：flow 表记录**可观测**的那些量 —— 每条流的字节与调用次数、
生命周期（建连/接受/关闭）、`socket.timeout()` 给的 RTO（**作为 RTT 的代理量，标注为 RTO 而非
RTT**）。未发送字节与窗口、socket 阻塞时间本轮不做：前者是瞬时量而非计数，
放进计数表只能是"最后见到的值"；后者要挂在 readiness/waker 路径上，是另一条线。
RTT 与重传待 smoltcp 那侧有接口可读时再补。

协议侧饱和信号（第 2 点）与 flow 表（第 3 点的替代口径）已实现，
形状与验证见 tracker 的"阶段四"一节。

## 四、组件边界

`ax-net` 与驱动是可复用组件，**不得依赖 StarryOS 内核对象**（`KernelTraceAux`）。
因此采用"单接口 + 无操作默认实现"的 observer：

- 组件侧定义**一个**统一接口（如 `emit(event: NetEvent)`，`NetEvent` 是 `#[repr(C)]` 枚举），
  默认实现为空操作，通过一个静态安装点替换。
- 内核侧（`os/StarryOS/kernel/src/tracepoint/`）实现该接口，把每个变体映射到对应的
  `define_event_trace!` 发射器。**tracepoint 定义集中在这一个适配层**，组件侧不出现
  任何 `ax-tracepoint` 依赖。
- eBPF 侧按固定字节偏移读 payload；两侧共享一份 `-common` crate 定义偏移与字段常量
  （参照 `apps/starry/ebpf/sched_trace` 的做法）。

这样做的代价是每个边界一次 Acquire 读加一次匹配与调用；好处是调用点数量等于契约里的事件数，
不再有 per-driver trait + FFI + glue 那一层（历史被否决的是那一层，不是 observer 模式本身）。

## 五、数据结构

- `PerCpuArray<u64>`：计数与直方图桶，规避跨 CPU 原子竞争。首选。
- 直方图：log2 桶覆盖 `(2^(b-1), 2^b]`，另留 sum 槽算均值；已知窄范围的现象可用固定桶细看。
- 有界 `HashMap`：仅用于载体到不了、确实无法在核内给出 duration 的关联，容量必须有上限。
- ringbuf：慢事件与错误事件的明细出口。阶段零已验证其 mmap 布局与 libbpf 一致、aya 消费者可用；rearm 竞争已走这条路（`slow_event kind=queue_rearm ...`），ringbuf 拒收时计 `ringbuf_dropped`——观测系统自身的丢失率。
- `CONFIG` map：采样率、慢事件阈值、启用接口与层级。
- `HEALTH` map：map miss、关联失败、无效事件、实际采样数、ringbuf 丢弃数。

两条硬要求：直方图必须同时输出 count / sum / 丢失数 / 实际采样率；标签只允许
`ifindex × queue × direction × reason`，五元组与 PID 只进按需开启的 LRU flow 模式。

## 六、构建与部署

沿用现有链路，不引入新工具链：

| 组件 | 要求 |
| --- | --- |
| eBPF 侧工具链 | 默认 `nightly`（aya-build 使用），需 `AYA_BPF_TARGET_ARCH=riscv64` |
| `bpf-linker` | 0.11.x，内嵌 LLVM 版本必须能读上述 nightly 的位码 |
| aya | 钉在走 `perf_event_open` 的 rev（`BPF_LINK_CREATE` 尚未实现） |
| 交叉工具链 | `/opt/riscv64-linux-musl-cross`，loader 编成静态 musl |
| 部署 | loader 与 eBPF 对象作为单一静态二进制注入 rootfs |

## 七、测试与验收

分层：

| 层 | 内容 |
| --- | --- |
| 单元 | 分桶边界、payload 编解码、reason 映射、指令预算的计数逻辑 |
| QEMU | 阶段零至三的主要验证场所，见下 |
| 实板 | 仅三类内容，见下 |

**QEMU 足以支撑阶段零至三的机制开发。** 原因是这阶段的观测对象是栈侧行为，
而 QEMU 的 virtio-net **是真实设备路径**（有 IRQ、DMA、队列、多队列），不是 loopback——
历史那条"loopback 不能代替设备路径"的教训对它不适用。现有基建已具备：
`test-suit/starryos/qemu/system/qemu-*.toml` 挂 virtio-net、`apps/starry/qemu/dual-net`
（双 virtio-net）、`qemu-e1000`（带 host HTTP server 的并发下载）。

QEMU 还提供两件板上做不到的事：**跨架构回归**（riscv64 与 aarch64 都跑，本分支已如此做过）
和**调不动时的取样**（E3 的卡点就是靠 QEMU 的 gdb stub 连续取样、发现 pc 停在
`trap_vector_base` 才定位到块映射拆分自毁的；那次的串口日志恰好不输出）。

**要上板的是两类**（`wifi:control` 与 `wifi:sdio_xfer` 已移出范围，见阶段二）：

1. 板子特有的现象——例如历史上那个 20–50 ms 时间片桶。
2. **探针开销的量化**——QEMU 的 virtio 吞吐特性与真实 SDIO/WiFi 差得太远，
   19% 那种数字在 QEMU 上没有意义。开/关对比必须在板上做。

需要警惕的一点：QEMU 验证的是**机制**，不是**驱动特性**。凡是与具体驱动相关的结论
（SDIO 事务时长、WiFi 控制面慢路径），QEMU 一律不能替代实板。

验收口径（与 `netmon-decision.md` §七一致，此处只列本方案独有的判据）：

| 判据 | 状态 |
| --- | --- |
| 阶段零三个缺口各一次"故意触发"的验证 | 已完成（`fault=3 bad_ok=0 good_ok=3` / `unbounded=rejected steps=0` / `ringbuf=0x5a5a1234deadbeef`，另有反证轮） |
| 阶段二每个事件在 QEMU 下计数非零 | 栈侧四个事件里三个计数非零；`queue_backpressure` 接通但该负载不触发（纯下载填不满 TX ring） |
| 阶段三五个区间各自产出分布 | 已完成（中位桶 18/19/20/21/19） |
| 三组对照可比 | 两组已做（1 与 1/16，五个区间中位桶全部相差不超过一个桶）；off 组属板上量化。此前记的『全采样 vs 1/64』作废——那一版里采样旋钮写不进去，两组速率相同 |
| 撤除 6 处 `#[inline(never)]` 并量出内联损伤 | 未做，见待办 |

## 八、风险

| # | 风险 | 缓解 | 现状 |
| --- | --- | --- | --- |
| R1 | 解释执行 + 无 verifier + 无指令预算 | 加载期拒绝回边与非 helper 调用，把"跑多久"变成静态可证的上界 | 已解决 |
| R2 | payload 布局是两侧隐式 ABI，加字段会移动偏移 | loader 在 attach 前解析 `format` 核对每个偏移，对不上就拒绝 | 已解决（`queue_irq` 的 `u64` 对齐错误就是它挡下的） |
| R3 | 载体字段要改可移植驱动接口，波及所有驱动 | 载体放 `DmaBuffer`（驱动只移动令牌），改 `ITxQueue::reclaim` 签名才会真的波及 | **不成立** |
| R4 | tracepoint 关闭态不是 NOP，而是"指针读 + 门控分支 + 调用" | 把逐帧报告整体压到采样分支之后：逐帧只留计数器自增、采样率读、掩码判断与一次载体存储，时钟只在被采样的帧上读；周期数留待板上量 | 结构已定形并在构建产物上核对（跟踪文档 P11）；周期数**未测** |
| R5 | 标签基数失控 | 默认指标只允许四个维度；五元组进按需 LRU 模式 | 仍适用 |
| R6 | 采样对分布形状的影响 | 同负载两组对照标定，不假设无偏 | 已标定：1 与 1/16 两组，五个区间中位桶均在一个桶内 |

## 附 A：现有分支提交的去留

本分支尚无 PR（`gh pr list` 为空），因此结构调整没有外部可见成本。
本方案自身的 15 个提交见 `README.md` 的进度表。

| 提交 | 内容 | 去留 | 理由 |
| --- | --- | --- | --- |
| `15998d6eb` | kprobe 注册失败返回错误而不是 panic 内核 | **保留** | kprobe 仍是排障路径；且这是独立的正确性修复 |
| `1a8bf237b` | 可执行内核区按基页映射（`axmm`） | **保留** | 修的是块映射拆分自毁，任何改内核文本权限的路径都会踩到，与路线无关 |
| `ba70cffb0` | 6 处 `#[inline(never)]` 挂点注解 | **阶段二完成后撤除** | 它只为 kprobe 保符号而存在；撤除本身是量内联损伤的对照实验。在此之前保留，板上冒烟还要用它验证链路 |
| `05d5cf6e6` | netmon 监测程序 | **保留，改定位** | 作为链路原型与 `netmon debug --kprobe <symbol>` 排障通道；默认观测接口由本方案承担 |
| `3065b5c1b` | 冒烟判据匹配 CRLF | 保留 | 冒烟基建 |

结论：**当前不需要回退任何提交。** 唯一计划撤除的是 `ba70cffb0`，且撤除时点排在阶段二之后，
不是因为它是错的，而是因为它的存在理由会被 tracepoint 取代。
