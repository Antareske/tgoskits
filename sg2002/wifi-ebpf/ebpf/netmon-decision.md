# StarryOS 网络栈 eBPF 监测：结论与方案路线

交叉核对了两份独立结论——本目录 `netmon-research.md` 的调研，与 `gpt-opion.md` 的方案建议。
两者的主线判断一致，分歧集中在少数几个具体机制上。综合后的建议是：

> 内核原生累计计数器作为事实来源，静态 tracepoint 作为可版本化的观测契约，
> 每包上下文随包携带而不在 BPF 侧配对，eBPF 只负责聚合、采样与临时诊断，
> kprobe 降级为开发排障工具。

这比"对一批 Rust 函数挂 kprobe"可靠得多，也比"只建 tracepoint 契约"更省——
因为把上下文塞进包里，跨层关联就不需要任何 BPF 侧状态。

## 一、交叉对比

### 1.1 一致的部分（直接采纳，不再论证）

| 结论 | 说明 |
| --- | --- |
| 不把网络统计全塞进 eBPF | `/proc/net/dev` 是包/字节/错误/丢包的权威口径，eBPF 不建第二套累计统计 |
| 静态 tracepoint 作为契约，kprobe 只做 debug | kprobe 保留为 `netmon debug --kprobe <symbol>`，不再作默认接口 |
| 先把 `NetQueueStats` 导出去 | 12 个字段每事件都在算却拿不到（`QUEUE_RUNTIME` 私有），这是"已有数据没有出口" |
| 直接携带 duration/result | 不在 BPF map 里猜 entry/return 配对 |
| 定长、无指针、版本化的事件结构 | 含 version/size/kind，loader 拒绝不支持的版本，不允许静默产出全零指标 |
| reason 用有限枚举 | 热路径不放字符串；`NetDeviceError` 已经是现成的有限枚举 |
| 第一版不依赖 BTF / CO-RE / `BPF_LINK_CREATE` | 这些在 StarryOS 都还没有 |
| ringbuf 只输出慢事件、错误、抽样明细 | 普通包不逐包上报 |
| 观测系统自身的丢失率是一等指标 | 否则 p99 好看可能只是慢事件被丢掉了 |
| 标签基数受限 | `ifindex × queue × direction × reason`；五元组/PID 只进按需开启的 LRU flow 模式 |
| loopback 不能替代设备路径 | 它绕过了物理队列、DMA 与中断 |
| schema 与 loader 同仓同版本维护 | Linux 官方明确 tracepoint 与 kprobe 挂点**都不是**稳定 ABI |

### 1.2 需要补强的部分（方向一致，但机制要落到具体位置）

**（1）"关闭态开销接近零"今天不成立。**

GPT 把它写进验收标准，但 `ax-tracepoint` 的关闭态是 `AtomicBool` 的 `Acquire` 读加一个
不跳转的分支，**没有 static key、没有 jump label、不打补丁**（这是该 crate 相对前身
`ktracepoint` 的有意取舍）。Linux 的做法是两套独立机制：

- **门控**用 static key：关闭态编译成一条 5 字节 NOP 走直线，
  没有内存读、没有分支、没有调用；
- **分发**用 static call：启用后不是间接调用，且**只挂一个探针时直接指向那个探针函数**
  （BPF 程序只是 `->funcs` 里的一个条目，单工具部署天然吃到这条快路）。

本项目正是一个监测器，天然落在"单探针直接调用"这条路上。所以关闭态该不该做补丁式
实现，是个框架级决定，**应当先量一次再定**——crate 的设计文档留的也是这个口子。

补充一个在构建产物上观察到的事实：本分支的构建里，`ax_net::observe::emit` 对观察者指针
仍保留非空判断，但调用目标被折叠成对该函数的直接调用——该静态量全程序只有一个常量写入点。
所以启用态的入口不是间接调用。这是"单观察者"这一设计的副产品，不是编译器给出的保证：
再加一个观察者写入点就会退回间接调用。

**（2）载体的具体位置。**

GPT 说"`packet_id`/`queue_id`/`operation_cookie` 必须显式传递"，方向对，但没落到类型上。
StarryOS 里三个现成的载体位置：

| 位置 | 类型 | 覆盖段 |
| --- | --- | --- |
| 协议栈内 | smoltcp `PacketMeta { id: u32 }` | 栈内各层之间（已在用，不建侧表） |
| TX 到驱动 | `TxSubmitOptions`（`drivers/interface/rdif-eth/src/lib.rs:893`） | 协议 → 队列 → 驱动，本来就每包一次穿过 |
| RX 自驱动 | `RxCompletion`（同文件 `:994`） | 驱动产出 → 上浮到协议 |

注意**不要**加在 `ProtocolEthernetFrame` 上：它是"非 DMA 端口与测试用的兼容内联帧"，
真正的队列路径不经过它（`transmit_frame_with_options` 直接填 DMA 存储）。

代价是这两个类型在可移植驱动接口 crate 里，加字段会波及所有驱动。设计上应**可选、有默认值**
——驱动不填就报 0，监测退化为"没有跨层数据"，而不是给出错误数据。

**（3）IRQ→poll 不是一个良定义的配对。**

一个 poll 可以合并多个 IRQ，所以"IRQ 到 poll 的唤醒延迟"在一对多的情形下没有唯一解。
应按 group 的 `sequence`/`generation` 建模，统计的是"一个 poll 覆盖了几次 IRQ、
它们各自的等待分布"，而不是一个配对时长。**这直接否掉了现有 netmon 的
`hist_irq_poll` 语义**，也解释了它为什么只能在 `--once` 下给出零值而没有暴露问题。

**（4）开销量级（用于判断代价是否可接受）。**

kprobe ~137 ns/次，静态 tracepoint ~30–50 ns，fentry ~24 ns，`tp_btf` ~15 ns。
这些来自不同测量环境，**绝对值只作参考，量级排序可靠**：kprobe 比静态 tracepoint
贵一个量级出头，差距就是"陷阱 + 寄存器全量捕获"与"一次直接跳转"。netstacklat 的
0.81% CPU 不是 fentry 快带来的，而是三个设计选择：每包只打一次时间戳、核内直方图聚合、
挂点少而只在层边界。

### 1.3 需要修正 GPT 结论的部分

**（1）分发时并不持有快照锁。**

这是两份结论共同的误读。`KernelExtTracePoint::acquire_snapshot()`
（`os/StarryOS/kernel/src/tracepoint/registry.rs:96`）取 `IrqMutex`、给读者计数加一、
克隆 `Arc`，**在返回租约前就 drop 了锁**；`read()` 的注释是"without retaining the raw
snapshot gate"。BPF 程序在"epoch 固定的 `Arc` + 读者计数"下运行，不在锁内。

所以热路径**可以直接发事件，不需要 per-CPU 环加 worker 重放**。剩下的代价只有两件小事：
`acquire_snapshot` 里那段短临界区，以及在调用点上下文里跑解释器的耗时。

**（2）"重复探测 `/proc/net/dev`"已经不是当前状态。**

`count_tx`/`count_rx` 两处探针已在本分支撤掉，注解面从 8 处收敛到 6 处
（只留队列边界、IRQ/poll、SDIO CMD53、WiFi 控制面）。这条批评针对的是更早的版本。

**（3）ringbuf 的可用性。**（已解决）

树内有 `BPF_MAP_TYPE_RINGBUF` 的实现，但此前没有任何 app 用过。端到端验证做过了：
kbpf 的页布局与 libbpf 一致（页 0 消费位、页 1 生产位与数据位、数据页双重映射），
aya 的消费者直接读得到记录（`ringbuf=0x5a5a1234deadbeef`）。

**（4）两个安全缺口必须先补，不是可选项。**

- `bpf_probe_read` 是**裸 memcpy**，不是故障安全读——`os/StarryOS/kernel/src/perf/nofault.rs`
  已经存在，但没有接进 helper 4。一次坏指针就是内核崩溃。
- **没有任何指令预算**：rbpf 里没有指令计数，也没有 Linux 那样的有界循环检查。
  **BPF 程序里一个循环会在 IRQ 上下文里永久自旋。**

在解释执行、无有效 verifier、`register_allowed_memory(0..u64::MAX)` 关掉了 rbpf 自身
边界检查的前提下，这两条比"多加几个探针"重要得多。

## 二、整体架构

观测数据分成三类，各有明确的所有者与出口：

| 数据 | 所有者 | 出口 | 用途 |
| --- | --- | --- | --- |
| 包、字节、错误、丢包累计值 | ax-net / 驱动 | `/proc/net/dev`、netlink | 权威长期指标 |
| IRQ、预算耗尽、ring 满、rearm race、CPU 归属 | queue runtime | 结构化快照（`/sys/kernel/debug/net_queue` 或 netlink dump） | 饱和与异常判断 |
| 时延分布、跨层关联、慢事件 | eBPF | per-CPU map（+ 待验证的 ringbuf） | 性能定位 |

StarryOS 已有的基础（已逐条核对到文件与行）：

- `NetDevStats`（`net/ax-net/src/router.rs:74`）维护 Linux 口径的 L2 包、字节、错误、丢包，
  8 个真实字段，没必要用 eBPF 重复计数。
- `NetQueueStats`（`net/ax-net/src/queue_runtime/state.rs:11`）已含 `irq`、`missed`、
  `budget_exhaustion`、`spurious`、`rearm_race`、`probe_deferred`、`poll_batches` 与
  `owner_cpu`/`last_irq_cpu`/`last_poll_cpu`。
- `NetworkQueueRuntime::stats()`（`net/ax-net/src/queue_runtime/mod.rs:307`）已有快照入口，
  缺的只是把它接到诊断接口。
- `ax-tracepoint`（`components/ax-tracepoint/`）是完整的 Linux `TRACE_EVENT` 形状框架：
  宏、`.tracepoint` 链接段、registry、filter 引擎、tracefs 的
  `id`/`format`/`enable`/`filter`/`trace_pipe`。cooked 与 raw 两条 BPF attach 路径都已跑通
  （`mytrace`、`rawtp`、`sched_trace` 三个 app 验证过）。**网络路径上目前一个 tracepoint 都没有。**
- `bpf(2)` 只实现部分命令（`os/StarryOS/kernel/src/ebpf/mod.rs:243`），
  无 BTF、无 `BPF_LINK_CREATE`、无 pin，因此第一版不应依赖这些。

## 三、探针契约

不要把 Rust 函数签名当 ABI。定义版本化、定长、无指针的网络事件：

| tracepoint | 关键字段 | 主要指标 | 频率 |
| --- | --- | --- | --- |
| `net:protocol_poll` | cpu、generation、duration、work count | 协议核心繁忙度、串行瓶颈 | 高 |
| `net:route_result` | ifindex、方向、reason | no-route、MTU、ARP、backpressure 等丢弃 | 低（错误） |
| `net:frame_enqueue` | packet_id、ifindex、queue、方向、len、carrier_ts | 帧进入跨 CPU 队列 | 高 |
| `net:frame_dequeue` | packet_id、queue、carrier_ts | queue residence time | 高 |
| `net:queue_irq` | queue、cpu、cause、sequence | IRQ 数、合并效果 | 中 |
| `net:queue_poll` / `net:queue_poll_irq` | queue、duration、work、blocked；另带本轮吸收的中断数与等待时长 | poll 时长、预算耗尽、一轮覆盖了几次中断 | 中 |
| `net:queue_backpressure` | queue、ring、reason | RX/TX ring 满、DMA token 缺乏 | 低（错误） |
| `net:queue_rearm` | queue、pending、race | rearm race、虚假中断 | 低（错误） |
| `net:driver_submit` | queue、cookie、方向、bytes | 提交速率 | 高 |
| `net:driver_complete` | cookie、status、duration | DMA/设备时延与错误 | 高 |
| `wifi:sdio_xfer` | direction、bytes、duration、result | AIC8800 CMD53 性能 | 中 |
| `wifi:control` | operation、duration、result | STA/AP 重配慢路径 | 低 |

关键原则：

1. **直接携带结果或 duration。** 能在状态所有者处算出来的，不要交给 BPF 用
   entry/return 去猜。前者的正确性可以单元测试，后者不能。
2. **按频率分层。** 错误与慢事件（`route_result`、`queue_backpressure`、`queue_rearm`、
   `wifi:control`）罕见，payload 可以给足，开销可忽略；逐包事件（`frame_*`、`driver_*`）
   高频，只带 id 与 carrier，测时延靠载体而不是靠 payload。
3. **跨层关联使用随包携带的上下文，而不是 map 配对。** 载体位置见 §1.2(2)。
   只有在载体确实到不了的地方（如无包的纯控制事件）才用有界 map。
4. **reason 使用有限枚举。** `NetDeviceError` 已经是现成的：
   `Again`（ring/DMA token 耗尽，即背压）、`Io`、`NoMemory`、`InvalidParam`、`Stopped`。
   直接把枚举值放进 payload，不要把字符串放进热路径。
5. **事件结构版本化。** 含 version、size、kind；loader 遇到不支持的版本拒绝加载，
   不能安静地产生全零指标。
6. **组件侧不要依赖内核对象。** `ax-net` 与驱动是可复用组件，不应直接引用 StarryOS 的
   `KernelTraceAux`。走"无操作默认的 observer 接口"，由内核侧安装真正的发射器。

## 四、eBPF 侧的数据结构

- `PerCpuArray<Counter>`：高频计数器，规避跨 CPU 原子竞争。首选，也是最省的。
- `PerCpuArray<u64>`：log2 时延直方图，桶覆盖 `(2^(b-1), 2^b]`，另留 sum 槽算均值。
- **有界** `HashMap`：仅用于载体到不了、确实无法在核内直接给出 duration 的关联。
  容量必须有上限。
- **ringbuf**：慢事件、错误事件与抽样明细。其 mmap 布局与 libbpf 一致、aya 消费者可用
  （阶段零已验证）；rearm 竞争已走这条路，拒收时计 `ringbuf_dropped`。
- `CONFIG` map：采样率、慢事件阈值、启用接口、启用层级。
- `HEALTH` map：map miss、关联失败、无效事件、实际采样数、ringbuf 丢弃数。

两条硬要求：

- **必须把观测系统自身丢了多少数据作为一等指标。** 否则 p99 很漂亮，可能只是慢事件
  被丢掉了。
- **标签只允许有限维度**：`ifindex × queue × direction × reason`。五元组、PID、端口、
  socket cookie 只能进按需开启的 LRU flow 模式，不能进默认指标。

另外，在写任何新程序之前先补两个缺口（见 §1.3(4)）：**故障安全的 `probe_read`** 与
**指令预算**。它们不是本项目专属，是整条 eBPF 路径的暴露面。

## 五、现有 netmon 原型的问题

现有 kprobe 版 netmon 可以继续作为验证链路的原型，但不适合作为最终架构：

- 依赖 Rust 符号、泛型单态化、内联决策与函数入口对齐。
- 为保留 kprobe 符号需要 `#[inline(never)]`，**反向影响生产热路径的内联优化**——
  而且没挂探针时也在付这个代价。
- 单槽 `Array<u64>` 存时间戳，多 CPU、嵌套调用、TX/RX 交错都会覆盖。
- `TS_PORT` 被 TX 与 RX 共用，无法严格保证 entry/return 配对。
- `hist_irq_poll` 建立在"IRQ 与 poll 一对一"的隐含假设上，而实际上一个 poll 会合并多个 IRQ。
- loopback 绕过物理队列、DMA 与中断，只能证明 loader 工作，不能证明设备路径指标正确。
- 每层计数与 `/proc/net/dev` 有重叠的部分（已撤掉部分探针，但口径仍需明确）。

迁移到 tracepoint 之后，那 6 处 `#[inline(never)]` 应当撤掉。**撤掉前后各跑一轮同样的
负载，这本身就是一次有价值的对照实验**——它能量出"内联损伤"到底值多少吞吐。

## 六、落地顺序

### 阶段零：补安全缺口，不增加新能力（已完成）

- 让 `bpf_probe_read` 读失败返回错误而不是崩溃。**实际做法**：不是接 `perf/nofault.rs`
  （那是 aarch64 专用的页表走读，SG2002 是 riscv64），而是基于异常表写一份四架构通用的
  内核地址安全拷贝。
- 让失控程序被拒绝而不是在 IRQ 上下文自旋。**实际做法**：rbpf 的解释循环没有插计数的
  地方，改为加载期拒绝回边与非 helper 调用——比运行期预算更强，且探针路径零成本。
- 验证 ringbuf 的 mmap 布局能被 aya 侧正常消费。（结论：可用）

### 阶段一：把已有数据接出来（已完成）

- 保留 `/proc/net/dev` 为接口累计统计的事实来源。
- 把 `NetQueueStats` 导出成 `/sys/kernel/debug/net_queue`，更长远的形态是结构化 netlink dump。
- 速率由用户态 loader 计算，内核不维护"每秒值"。
- 建立基线：吞吐、PPS、CPU、IRQ、poll batches、budget exhaustion。

### 阶段二：增加静态网络 tracepoint（栈侧四个事件已完成，差 `net:route_result` 与两个 wifi 事件）

优先实现低频与边界事件，它们开销可忽略、信息量最大：

- `net:route_result`、`net:queue_backpressure`、`net:queue_rearm`
- `net:queue_irq`、`net:queue_poll`（按 sequence/generation 建模，不配对）
- `wifi:control`、`wifi:sdio_xfer`

逐包事件（`frame_*`、`driver_*`）放阶段三，因为它们依赖载体先落地，
而且需要采样策略一起设计。

复用现有的 `ax-tracepoint` 注册体系（`os/StarryOS/kernel/src/tracepoint/mod.rs:53`）。
ax-net 与驱动层通过无操作默认的 observer 接口接入，不直接依赖内核对象。

### 阶段三：跨层关联与载体（已完成）

- 载体放在 **`DmaBuffer`** 上而不是 `TxSubmitOptions`/`RxCompletion`：令牌是唯一走完全程的
  对象，驱动只移动它，所以四个驱动一行没改，也不必假设完成按提交顺序返回。
- 只对 1/64 的帧填充载体（可运行时调整），计算：
  - IRQ → queue poll
  - queue poll → RX publish
  - RX publish → protocol consume
  - protocol TX → queue submit
  - queue submit → DMA completion

**这一阶段才回答"慢在哪里"**，而不只是"某个函数执行了多久"。

### 阶段四：按需 flow / socket 诊断

增加有限容量的 LRU flow 表，支持 TCP RTT、重传、连接建立与 socket 阻塞时间。
由于当前是单 smoltcp protocol owner，首要目标是判断：

- protocol executor 是否饱和；
- socket workload 是否导致长 poll；
- queue owner 是否因 backpressure 或 SDIO 被拖慢。

不建议一开始复制 Linux 的 XDP / tc / skb / cgroup 全套模型——StarryOS 没有相同的
数据结构和并发拓扑。

## 七、验收标准

截至当前（阶段零、一、三在 QEMU riscv64 上完成，阶段二完成栈侧部分），逐条状态：

| 判据 | 状态 |
| --- | --- |
| tracepoint 关闭分支的开销有实测数字 | **周期数未做**。QEMU 的 CPU 是模拟的，周期数没有意义；这条与 Linux 的 NOP 基线一样只能在板上量。逐帧固定成本的结构已在构建产物上核对：未采样的帧只付计数器自增、采样率读、掩码判断与一次载体存储，报告路径整体在采样分支之后 |
| 加载监控后吞吐下降与 CPU 增量有上限 | **未做**，同上，属板上量化 |
| `/proc/net/dev` 始终是包/字节权威值 | 成立。eBPF 侧只读 `/sys/kernel/debug/net_queue` 的队列计数与事件，没有第二套包/字节累计 |
| 每个直方图输出 count / sum / 丢失数 / 实际采样率 | 已有。每个直方图带 `count` / `sum_ns` / `mean_ns`（sum 槽在桶块之后，同序排列），采样率由 `/sys/kernel/debug/net_sample_rate` 与 `count` 对照得出，观测系统自身的丢失数由 `ringbuf_dropped` 给出 |
| 撤掉 6 处 `#[inline(never)]` 并量出差异 | 未做（撤除排在阶段二之后，且它本身是一次对照实验） |
| SMP 下没有时间戳错配 | 机制上成立：载体随包走，不在共享槽里配对；中断时刻槽用 `compare_exchange` 只让第一个写者占位。板上多核仍需复验 |
| loopback / QEMU / 实板分别验证 | QEMU（virtio-net，真实设备路径）已完成；实板未做；loopback 未用（本就不该代替设备路径） |
| probe-off / on-unsampled / on-sampled 三组对照 | **两组已做**：1 与 1/16 两组，五个区间各自的中位桶相差都不超过一个桶。off 组属板上量化 |
| 用现有 network-throughput 与 ltp-netstress 作负载 | 未用。QEMU 用例用宿主 HTTP 服务器下载加一次上传，理由是 netstress 只有 loopback，替代不了设备路径 |
| 故意触发一次死循环 BPF 与一次坏指针读，确认内核存活 | **已完成**。`unbounded=rejected steps=0`；坏指针读返回 `-EFAULT` 且内核存活，另有一轮反证（换回裸 memcpy 时同一自检把内核打成 `Unhandled Supervisor Page Fault`） |

简而言之，StarryOS 最值得建设的不是"更多 kprobe"，而是一套**由网络状态所有者定义、
可版本化、可关联、可统计自身丢失率的静态网络事件契约**；在这套契约之上，再让
**每包上下文随包携带**，从而把跨层关联的状态从 BPF 侧彻底移走。eBPF 建立在这两者之上，
而不是依赖编译器碰巧保留下来的 Rust 符号。
