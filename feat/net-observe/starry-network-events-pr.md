# feat(ax-net): expose per-queue state and net:* tracepoint events

## 背景

Starry 网络栈此前没有对外的观测出口：队列运行时按 poll group 维护的身份与计数（owner CPU、IRQ、budget、rearm、RX 丢弃等）只存在于内部状态里，无法从用户态按组读出；队列执行器与协议执行器的关键状态转移（轮询、rearm、背压、帧交接、预算耗尽让出）也没有事件面。定位“某一轮为什么停在这里”“谁的接收环满了”“协议执行器是否在饱和”这类问题时，只能临时插桩，既不可复用也不可发布。同时，任何观测能力都不能改变网络行为本身。

本改动为网络栈补齐这两类出口，并让它们可开关、可被 tracefs 与 eBPF 消费。

## 方案

**事实归所有者，事件面归宿主。** 事实由拥有它的模块产生：队列身份与计数属于 `ax-net` 的队列运行时，协议执行器的调度转移属于协议执行器。`ax-net` 不依赖 StarryOS 或 `ax-tracepoint`；事件名称、记录格式与启用状态由宿主的事件适配模块（StarryOS `kernel/src/tracepoint`）定义。两侧通过一个**窄观察端口**交付事实。

**观察端口契约**：每事件一个端口实例，形态是一个类型化函数指针槽加一个已发布标志。安装一次、同一函数重复安装幂等、替换存活消费者视为不变量违背、端口不卸载；未安装等价于没有消费者，状态转换与结果不变。报告只读一个标志与一个槽，不分配、不读时钟、不取网络锁，可在队列执行器线程与协议执行器线程直接调用。

**启用事实只有一份**：仍由 `ax-tracepoint` 的每事件门控拥有，运行时只持有镜像。宿主在回调集合变化处把结果镜像到运行时的已发布标志——这一处同时覆盖 tracefs `enable` 与 perf/BPF attach 两条消费者通路，由门控 sink 注册表统一分发（事件模块在安装时注册自己的发布函数）。

**事件按“被报告的对象”命名**，字段只用固定宽度整数（宽度转换在适配层以饱和代替截断），结果码用 `#[repr(u32)]` 加常量断言钉成对外契约；事件可被关闭或丢弃，因此不代替 `NetQueueStats`、`net_queue_snapshots()` 或 `/proc/net/dev` 的累计。

## 改动点

**1. 队列身份与计数出口**

- 每个 poll group 在建组时固定不可变身份（设备发现序索引、group ID、owner CPU），随 group 一起经过启动缺席裁剪，之后不再变动；接口名在 `init_network` 发布接口时绑定，查询方按接口 ID 取名字。
- 新增 `NetQueueSnapshot` 与 `net_queue_snapshots()`：按 poll group 返回身份与计数，运行时尚未发布时返回空；`NetQueueStats` 的既有计数语义保持不变。
- 队列 RX 丢弃拆成两个同源事实：对外查询用的只增累计，与供接口折入的增量；接口 `rx_dropped` 口径不变。
- StarryOS 侧新增 debugfs `/sys/kernel/debug/net_queue`，逐组渲染身份与计数（诊断出口，非 ABI）。

**2. 事件面基础设施**

- `kernel/src/tracepoint` 新增 gate sink 注册表：事件模块在 `install()` 里注册自己的门控镜像发布函数，注册表在回调集合变化时统一回调；重复注册同一事件断言失败。sink 的契约（只发布一个原子状态、不阻塞、不分配、不 panic、不回调追踪层）写进设计文档。

**3. 六个 `net:*` 事件**

| 事件 | 触发边界 | 字段 |
| --- | --- | --- |
| `net:queue_poll_round` | 队列执行器一次 poll 调用返回，恰好一条（含提前返回与失败轮次） | 身份、budget、work_units、outcome（空闲/仍有工作/阻塞/失败） |
| `net:queue_rearm` | rearm 未以正常空闲结束时，一次至多一条 | 身份、outcome（竞态/仍有工作/延迟重试/失败） |
| `net:queue_backpressure` | 设备明确要求等待的可重试拒绝（TX 提交保留帧、RX 补投保留替换缓冲） | 身份、stage（TX/RX）、reason（重试/链路不可用） |
| `net:tx_submit` | 驱动接纳一帧（逐帧） | 身份、帧长 |
| `net:rx_publish` | 帧发布到协议侧环（逐帧） | 身份、帧长 |
| `net:proto_yield` | 协议执行器预算耗尽、让出 CPU 时 | owner CPU、reason（次数上限/时间上限/两者同时）、work_pending |

`net:proto_yield` 报告的是**让出转移**而非“一轮协议轮询”。预算有两个上限（连续轮询次数与经过时间），只在让出时重置，因此轻载下通常每个唤醒周期至多一条记录；判读需结合 `reason` 与 `work_pending`（`reason=0/2` 或 `work_pending=1` 才是协议侧饱和信号）。`queue_backpressure` 的 `(RX 补投, 链路不可用)` 组合按构造不可达（RX 补投对链路不可用走整轮失败），已在文档写明。

**4. 测试与用例**

- `ax-net` 单元测试覆盖每个事件的触发边界：一次触发恰好一条、提前返回不重复、失败轮次报告失败前的真实工作量、端口开关与有无消费者都不改变轮询结果；rearm 只报非空闲结局；背压只在可重试拒绝时出现且被拒帧不报为已提交；`tx_submit`/`rx_publish` 每次接纳/发布恰一条、发布被拒不报告；协议执行的预算判定在次数上限、时间上限与两者同时三种情况下分类正确。
- 系统用例 `qemu/system/net-queue`：读取 debugfs 记录，校验 16 个字段、接口名与 `/proc/net/dev` 一致、owner CPU 落在在线集合内。
- 系统用例 `qemu/system/net-events`：六个事件可发现且 `format` 声明文档化字段；启用后发真实数据报驱动队列轮询与协议推进，流量驱动的四个事件必须出现自洽记录（结果码在取值范围内、工作量不超过预算、帧长与待办标志合法），等待条件是“全部出现或有界超时”；关闭清空后同样流量不再产生记录。
- eBPF 冒烟 app `apps/starry/ebpf/net_queue_poll`：附着 `net:queue_poll_round` 并读回记录，只证明 `load → attach → enable → read` 连通，不定义事件语义。

**5. 文档**

- `docs/design/ax-tracepoint.md` 新增「新增事件：约定与责任划分」表：事件定义位置、命名空间、字段类型、宽度转换、arity 豁免边界、门控 sink 契约、跨 crate 观察端口、上下文、成本口径与验收要求。
- `docs/docs/architecture/net/` 下新增 `events.md`（事件契约：端口语义、逐事件字段与结果码、三态成本、事件准入、验收与覆盖边界），并同步 `api.md`（端口与快照 API）、`devices.md`、`testing.md`、`architecture.md`、`queue-napi-runtime.md`、`integration.md` 与 `driver/runtime.md` 中相关段落。

## 测试结果

以下为最终一轮验证（本地 QEMU 与静态检查）：

- `cargo xtask clippy`：`ax-net` 9 项、`starry-kernel` 80 项（feature/target 组合）全部通过。
- `cargo xtask test --since dev`：15 个受影响软件包的标准库测试全部通过，其中 `ax-net` 149 项单元测试。
- `cargo xtask starry test qemu -c qemu/system/net-queue`：四种 QEMU 配置均报告 `groups=1 interfaces=1`。
- `cargo xtask starry test qemu -c qemu/system/net-events`：四种 QEMU 配置均通过；真实流量下读到队列轮询、帧接纳、帧发布与协议执行器让出记录（例如 `queue_poll_round budget=256 work_units=3 outcome=0`、`tx_submit frame_len=60`、`rx_publish frame_len=64`、`proto_yield reason=1 work_pending=1`），关闭事件后同样流量不再产生记录。
- `cargo xtask starry app qemu -t ebpf/net_queue_poll`：x86_64 通过，附着链路读回记录。

## 未验证与限制

- **覆盖边界**：`queue_rearm` 的非空闲结局与 `queue_backpressure` 需要设备真的竞态或真的忙，QEMU 不保证触发，这两者的语义由单元测试覆盖，系统用例只保证装配与格式；`proto_yield` 的触发环路在协议执行器线程内，host 单元测试够不到，其真实触发由系统用例证明。
- **已记录的缺口**：TX 提交返回永久错误时帧被回收放回空闲池，既不报接纳也不报背压，当前也没有队列侧计数；文档已写实这一口径，是否补计数或另立事件留待后续按事件准入评估。
