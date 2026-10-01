
专业方案不应把“网络统计”全部塞进 eBPF。对 StarryOS，我建议采用：

  > 内核原生累计计数器作为事实来源，静态 tracepoint 作为稳定观测契约，eBPF 负责聚合、采样和临时诊断，
  > kprobe 只用于开发排障。

  这比“对一批 Rust 函数挂 kprobe”更可靠，也更符合 StarryOS 当前网络栈的所有权模型。

  ## 一、整体架构

  应用 syscall
     │
  TCP / UDP / raw socket
     │
  唯一 net-protocol executor
     │
  Router / smoltcp
     │
  QueueFramePort                    控制面
     │                              └─ DHCP / Wi-Fi transaction
  SPSC frame/token rings
     │
  net-queue-cpuN executor
     │
  DMA queue / IRQ / rearm
     │
  VirtIO / E1000 / RTL8125 / AIC8800 / SDIO

  观测系统分成三类数据：

   数据                                  所有者           输出方式                     用途
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━
   包、字节、错误、丢包累计值            ax-net/驱动      /proc/net/dev、netlink       权威长期指标
  ────────────────────────────────────  ───────────────  ───────────────────────────  ────────────────
   IRQ、预算耗尽、ring 满、rearm race    queue runtime    结构化快照                   饱和与异常判断
  ────────────────────────────────────  ───────────────  ───────────────────────────  ────────────────
   时延分布、跨层关联、慢事件            eBPF             per-CPU map + ring buffer    性能定位

  StarryOS 已有比较好的基础：

  - NetDevStats (net/ax-net/src/router.rs:67) 已经维护 Linux 口径的 L2 包、字节、错误和丢包，没必要再
    用 eBPF 重复计数。

  - NetQueueStats (net/ax-net/src/queue_runtime/state.rs:9) 已经包含 IRQ、missed、budget exhaustion、
    spurious、rearm race 和 CPU ownership 信息。

  - NetworkQueueRuntime::stats() (net/ax-net/src/queue_runtime/mod.rs:307) 已具备快照入口，首先应把它
    接到诊断接口。

  - StarryOS 已支持 cooked/raw tracepoint、perf event、常用 BPF map；BPF map fd 也已经具有 ring buffer
    所需的 mmap/poll 基础，见 os/StarryOS/kernel/src/ebpf/map.rs:25。

  - 当前 bpf(2) 分发 (os/StarryOS/kernel/src/ebpf/mod.rs:243) 只实现部分命令，因此第一版不应依赖 BTF、
    CO-RE、BPF_LINK_CREATE 等尚未完整支持的能力。

  ## 二、建议的探针契约

  不要直接把 Rust 函数签名当 ABI。定义版本化、定长、无指针的网络事件：

   tracepoint                关键字段                                   主要指标
  ━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
   net:protocol_poll         CPU、generation、duration、work count      协议核心繁忙度、串行瓶颈
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:route_result          ifindex、方向、结果原因                    no-route、MTU、ARP 等丢包
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:frame_enqueue         packet_id、ifindex、queue、方向、长度      帧进入跨 CPU 队列
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:frame_dequeue         packet_id、queue、等待时长                 queue residence time
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:queue_irq             queue、CPU、cause、sequence                IRQ 数、合并效果
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:queue_poll            queue、CPU、duration、work、budget flag    IRQ→poll、poll 时长、预算耗尽
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:queue_backpressure    queue、ring、reason                        RX/TX ring 满、DMA token 缺乏
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:queue_rearm           queue、pending、race                       rearm race、虚假中断
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:driver_submit         queue、cookie、方向、bytes                 提交速率
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   net:driver_complete       cookie、status、duration                   DMA/设备时延和错误
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   wifi:sdio_xfer            direction、bytes、duration、result         AIC8800 CMD53 性能
  ────────────────────────  ─────────────────────────────────────────  ───────────────────────────────
   wifi:control              operation、duration、result                STA/AP 重配慢路径

  几个关键原则：

  1. 直接携带结果或 duration
     能在状态所有者处计算时，不用 kprobe entry/return 在 BPF map 中猜配对关系。

  2. 跨层关联使用稳定 ID
     packet_id、queue_id、operation_cookie 必须显式传递。不要用裸指针作为长期关联键。

  3. reason 使用有限枚举
     例如 NoRoute、MtuExceeded、RxRingFull、DmaError。不要把字符串放进热路径。

  4. 事件结构版本化
     包含 version、size、kind，loader 遇到不支持的版本应拒绝加载，不能安静地产生全零指标。

  Linux 本身也明确指出 tracepoint 和 kprobe 挂载位置都不是稳定 ABI，因此 StarryOS 应把事件 schema 与
  loader 放在同一仓库、同一发布版本内维护，而不是假设函数名永久稳定。Linux BPF Design Q&A

  ## 三、eBPF 侧的数据结构

  推荐：

  - PerCpuArray<Counter>：高频计数器，避免跨 CPU 原子竞争。
  - PerCpuArray<HistogramBucket>：log2 时延直方图。
  - 有界 HashMap<(queue_id, sequence), StartState>：仅用于确实无法在内核侧直接给出 duration 的关联。
  - ring buffer：仅输出慢事件、错误事件和抽样明细；普通包不逐包上报。
  - CONFIG map：采样率、慢事件阈值、启用接口、启用层级。
  - HEALTH map：map miss、关联失败、ringbuf drop、无效事件、采样数。

  必须把“观测系统自身丢了多少数据”作为一等指标。否则 p99 很漂亮，可能只是慢事件被 ring buffer 丢掉了。

  Linux ring buffer 在空间不足时不会阻塞，生产者必须处理 reservation 失败；共享 ring buffer还能保留跨
  CPU 事件顺序。Linux BPF ring buffer 文档

  标签只允许有限维度：

  ifindex × queue × direction × reason

  五元组、PID、端口和 socket cookie 只能进入按需开启的 LRU flow 模式，不能进入默认指标，否则很快产生不
  可控基数和内存占用。

  ## 四、现有 netmon 原型的问题

  仓库历史中的 kprobe netmon 可以作为验证 eBPF 链路的原型，但不适合作为最终架构：

  - 依赖 Rust 符号、泛型单态化、内联和函数入口对齐。
  - 为了保留 kprobe 符号需要 #[inline(never)]，会反向影响生产热路径优化。
  - 使用单槽 Array<u64> 保存时间戳时，多 CPU、嵌套调用、TX/RX 交错都会覆盖。
  - TS_PORT 同时被 TX 和 RX 使用，无法严格保证 entry/return 配对。
  - IRQ 到 poll 不是天然一对一关系；一个 poll 可能合并多个 IRQ，应该按 group sequence/generation 建
    模。

  - loopback 绕过物理 queue、DMA 和 IRQ，只能证明 loader 工作，不能证明设备路径指标正确。
  - 重复探测 /proc/net/dev 已有的包字节计数，会产生两套事实来源。

  因此，kprobe 模式应该保留为：

  netmon debug --kprobe <symbol>

  而不是默认、长期稳定的监控接口。

  ## 五、最适合 StarryOS 的落地顺序

  ### 第一阶段：不增加新的 eBPF 能力

  - 保留 /proc/net/dev 为接口累计统计事实来源。
  - 把现有 NetQueueStats 导出成 /proc/net/queue，或更长期地设计结构化 netlink dump。
  - 用户态 loader 负责计算速率，不在内核维护“每秒值”。
  - 建立基线：吞吐、PPS、CPU、IRQ、poll batches、budget exhaustion。

  ### 第二阶段：增加静态网络 tracepoint

  优先实现：

  - queue_irq
  - queue_poll
  - queue_backpressure
  - frame_enqueue/dequeue
  - driver_submit/complete

  可复用现有 ax-tracepoint 注册体系 (os/StarryOS/kernel/src/tracepoint/mod.rs:53)。共享的 ax-net 和驱
  动层应通过无操作默认 observer/trace 接口接入，避免直接依赖 StarryOS 内核对象。

  ### 第三阶段：端到端采样

  只对例如 1/1024 的帧分配 packet_id，计算：

  IRQ → queue poll
  queue poll → RX publish
  RX publish → protocol consume
  protocol TX → queue submit
  queue submit → DMA completion

  这样才能回答“慢在哪里”，而不只是报告某个函数执行了多久。

  ### 第四阶段：按需 flow/socket 诊断

  增加有限容量的 LRU flow 表，支持 TCP RTT、重传、连接建立和 socket 阻塞时间。由于 StarryOS 当前是单
  smoltcp protocol owner，首要目标应是判断：

  - protocol executor 是否饱和；
  - socket workload 是否导致长 poll；
  - queue owner 是否因 backpressure 或 SDIO 被拖慢。

  不建议一开始复制 Linux 的 XDP、tc、skb、cgroup 全套模型；StarryOS 当前没有相同的数据结构和并发拓扑。

  ## 六、验收标准

  至少要证明：

  - 未加载 eBPF 时 tracepoint disabled 分支开销接近零。
  - 加载默认采样监控后，吞吐下降和 CPU 增量有明确上限。
  - /proc/net/dev 始终是包/字节权威值，eBPF 不建立第二套累计统计。
  - 每个直方图同时输出 count、sum、丢失数和实际采样率。
  - SMP 下没有时间戳错配；queue、IRQ 和 poll 的 CPU 与 owner 契约一致。
  - 分别在 loopback、VirtIO/E1000 QEMU、AIC8800 实板验证；loopback 结果不能代替设备路径。
  - 对比 probe-off、probe-on-unsampled、probe-on-sampled 三组相同负载。
  - 使用现有 network-throughput 和 ltp-netstress 作为工作负载，但新增设备路径断言和性能采样。

  简而言之，StarryOS 最值得建设的不是“更多 kprobe”，而是一套由网络状态所有者定义、可版本化、可关联、可
  统计自身丢失率的静态网络事件契约。eBPF 应建立在这套契约之上，而不是依赖编译器碰巧保留下来的 Rust 符
  号。