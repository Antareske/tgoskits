# StarryOS 网络事件实施交接方案

## 1. 定位与交付边界

`feat/net-observe` 定性为“网络事件引导的网络栈基础能力实现”。截至提交 `df705c9dd`，它已完成网络事件之前的基础能力：`NetQueueIdentity` 为 poll group 提供运行时内的身份，`NetQueueStats` 维护队列累计，`NetworkQueueRuntime::queue_snapshots()` 汇集快照，StarryOS 通过 `/sys/kernel/debug/net_queue` 查询这些事实。它还没有定义或触发 `net:*` tracepoint。下一阶段要让网络栈在真实状态边界报告事件；eBPF 仅验证既有附着链路能读取事件，不决定网络事件的语义、范围或字段。

### 1.1 现有事实与目标

队列累计回答“到目前为止发生了多少次”，事件回答“这一轮具体发生了什么”。事件可能关闭或丢失，因此不能代替 `PollGroupState::stats`、`/proc/net/dev` 或现有快照。每个新事件必须有独立的网络诊断问题、唯一的事实所有者、明确的触发边界和可解释的字段；不要求每个计数都有同名事件。

目标是先交付一条完整、稳定、可关闭的队列事件链：`ax-net` 的事实发生点 → StarryOS 的 `net:*` tracepoint → tracefs 的 `id`、`format`、`enable` → 至少一种真实消费者。用户应能在不开启事件时继续使用网络和查询累计，在开启事件时获得与实际队列运行一致的记录。目标工作树 `wt-feat-net-observe/www/` 中的 `starry-network-observability-plan.md` 和 `starry-network-events-explained.md` 是背景材料；本交接方案收紧其事件实施范围，不表示候选事件已完成。

实现时以目标分支的 `net/ax-net/src/queue_runtime/{state.rs,executor/mod.rs}` 为队列事实锚点，以 `os/StarryOS/kernel/src/tracepoint/mod.rs`、`components/ax-tracepoint/src/basic_macro.rs` 为登记和门控锚点，以 `os/StarryOS/kernel/src/perf/tracepoint.rs` 为附着及回调上下文锚点。所有路径均相对 `wt-feat-net-observe` 仓库根目录；当前工作树的来源分支实现不能代替目标分支现状。

### 1.2 明确不做

本阶段不迁移 `sg2002/wifi-ebpf` 的 `DmaBuffer::carrier`、逐帧采样器、`net_sample_rate`、为探针保留符号的 `#[inline(never)]`、`TcpSocket::flow_key` 或混合语义的 `net:flow`。不实现逐帧跨阶段时延、按帧关联表、常驻观测时间戳，也不因 eBPF 示例需要某个数值而改变队列或 DMA 所有权。来源分支的同名 `net:*` 事件只能作为边界线索；若字段或触发语义不同，不能直接复制其对外契约。

## 2. 事件范围与语义

先从 `QueueGroupExecutor::poll()` 的一次完成事件建立可审查的契约。现有 `GroupPollOutcome`、`PollGroupState::identity` 和 `queue_executor_main()` 已掌握结果与归属；增加事件时不需要另建队列统计表。其他候选事件按具体诊断缺口逐项纳入，不把原方案的整张候选表当成一次性任务清单。

### 2.1 首个事件：队列轮询完成

建议先定义 `net:queue_poll`：每次 `queue_executor_main()` 成功 `claim()` 后调用一次 `group.poll(budget)`，在该调用返回时恰好报告一次。它描述一次队列轮询调用，不包括之后 `finish_idle()` 内的 IRQ rearm。`Idle`、`More`、`Blocked` 和 `Failed` 都应有可区分的结果码；是否把启动期间未成功 `claim()` 的情况排除，按此定义应排除。事件不报告“耗时”，因为现阶段没有为该事件正当化开始时间与时钟成本。

下表是字段语义草案，实施前应固定整数宽度、范围转换、结果码和 `format` 布局，再使其成为对外契约。字段名可以调整，但不能改变所表达的事实。

| 字段 | 语义与约束 |
| --- | --- |
| `device`、`group` | 本次网络运行时内的设备发现序索引和驱动 group ID；二者组合定位 poll group，不承诺跨重启或设备重建稳定。宽度转换须检查，不能静默截断。 |
| `owner_cpu` | 建组时固定的队列归属 CPU；不是事件发生时任意采样的 CPU。 |
| `budget` | 传给本次 `group.poll()` 的 CPU 工作预算，单位是执行器工作单位。 |
| `work_units` | 本次已完成的执行器工作单位；现有 `work` 混合 TX 完成、提交、RX 回收等操作，不能标成帧数或包数。 |
| `outcome` | 固定数值对应 `Idle`、`More`、`Blocked`、`Failed`；不能把可重试或阻塞解释成丢包。 |

当前 `GroupPollOutcome::Failed` 没有携带 `work`，但 `poll()` 可能在已完成部分工作后失败。不得因此把失败事件的 `work_units` 填为零并宣称“未做工作”。实施时应让结果保留实际工作量，或定义明确的有效性标志；优先选择不改变队列算法、只补全报告信息的最小改动。事件触发点宜在调用者获得结果后、`finish_idle()` 等后续状态转换前，避免给 `poll()` 的多处提前返回重复插桩。

### 2.2 后续事件的准入

`queue_rearm` 可在首个事件稳定后评审：只报告 `rearm_and_check()` 返回“已有工作待处理”等明确结果，不把正常 `Idle`、定时重试和竞态混成一个原因。其事实位置在 `QueueGroupExecutor::finish_idle()`，并应区分事件所述的是 rearm 结果，而非整轮 poll 结果。

其余事件必须先解决下表中的语义或执行上下文问题，不能只因原方案列名就实施。

| 候选 | 实施前必须解决的问题 |
| --- | --- |
| `queue_irq` | `NetHardIrqResult::{Spurious, Schedule, ProbeDeferred}` 的边界与计数口径不同；若要覆盖全部 IRQ，应在分流结果处定义分类，而不能只把 `schedule_irq()` 调用都当成同一种 IRQ。硬 IRQ 触发时的 tracepoint、消费者和 eBPF helper 安全须单独审查。 |
| `queue_backpressure` | 设备层 `Again`、驱动层 `Retry`、TX ring 阻塞和 RX refill 阻塞并非同一事实；先选定一个确切层级与唯一结果，再决定是否拆成不同事件。 |
| `route_drop` | 无路由发生在出口接口确定之前，其他最终丢弃又分散在设备、loopback 与 RX 路径；不能承诺每条记录都有接口，也不能把 `Retry` 算作最终丢弃。应先限定一个最终丢弃边界及原因集合。 |
| `proto_poll`、TX/RX 交接事件 | 只有快照及首批队列事件不足以区分实际故障时才逐项增加；两条帧边界事件没有同帧身份时不能声称提供逐帧时延。 |

## 3. 分层与运行代价

`ax-net` 是队列事实的所有者，StarryOS 是 tracepoint 名称、记录格式和启用状态的所有者。`ax-net` 不依赖 StarryOS 或 `ax-tracepoint`；StarryOS 在 `tracepoint` 适配模块中把窄领域事件映射为静态 tracepoint。ArceOS 等不安装观察端口的使用者保持原有网络行为。不要为了事件建设通用全局事件总线、第二套队列状态登记表或长期兼容旧版来源分支字段。

### 3.1 观察端口

第一个事件只需要启动时安装、之后不替换的窄回调入口。回调报告已知的队列身份、预算、工作单位和结果；未安装回调相当于没有消费者，不影响状态转换。实现应说明谁在网络运行时和 IRQ 发布前完成安装、谁在关闭时阻止新调用及等待在途调用；不能只靠 `Arc` 保证业务状态仍有效。若使用全局函数指针或原子发布，需证明初始化顺序、可见性与重复安装行为，不把这些条件留给调用方猜测。

StarryOS 使用现有 `ax_tracepoint::define_event_trace!` 和 `tracepoint_init()` 注册事件，形成 `events/net/queue_poll/{id,format,enable}`。`ax-tracepoint` 的 `key_is_enabled()` 是回调 gate 的权威来源，不在 `ax-net` 维护一份可分叉的布尔开关。廉价且本来已计算出的字段可直接传至适配层，由生成的 `trace_queue_poll()` 再检查 gate；只有为观测额外计算的昂贵字段才需要提前查询同一个 gate。查询 gate 与真正触发之间允许启停竞争，事件函数自身仍须完成最终 gate 检查。

### 3.2 上下文与资源约束

事件回调同步运行于触发上下文。首个 `queue_poll` 位于队列执行器任务上下文，仍不得在持有网络锁时进入可反向获取该锁的回调；需检查调用链和 `trace_pipe`、perf 回调的实际约束。热路径不构造字符串、不分配、不读取仅供观测的时钟，不改变包令牌或队列结果。关闭事件时只承担必要的入口和 gate 检查成本；若测得退化，再用同一工作负载核对分支与回调成本，不以 `#[inline(never)]` 或额外采样状态回避问题。

`queue_irq` 暂不因已有 tracepoint 框架而默认安全：StarryOS 的 `TracepointPerfEvent::set_bpf_prog()` 注册的回调会在事件现场执行程序，硬 IRQ 下必须逐一审查解释器、helper、map、日志及错误路径是否禁止睡眠和分配。没有该证据时，先交付任务上下文事件；需要 IRQ 事实则另行设计安全的记录/延后机制及丢失口径，而不是把任意 eBPF 程序直接放进 IRQ 路径。

## 4. 验收与实施顺序

网络事件的正确性由事件契约和网络行为决定，eBPF 仅提供附着链路的最后一项证明。以 `feat/net-observe` 的现有计数、快照和测试为基线，分阶段保留可回退边界；每增加一个候选事件，都重新确认必要性、触发位置和运行成本。

### 4.1 实施步骤

以下顺序让语义问题先于宏展开、用户态加载器或性能调优得到解决。每一步完成后应能明确指出尚未交付的部分。

1. 固定 `queue_poll` 的一次触发定义、结果码、`work_units` 与失败后工作量口径，选择运行时内的队列身份字段和整数宽度。
2. 在 `ax-net` 增加最小观察端口及唯一报告点；保持无观察端口、事件关闭和事件开启时的队列状态转换一致。
3. 在 StarryOS 定义 `net:queue_poll`，检查注册、`format`、`id`、`enable` 和启停竞争；不复制来源分支的字段契约。
4. 用真实队列运行验证事件条数、身份、预算、工作单位及四类结果；同时核对快照和 `/proc/net/dev` 的累计口径未变化。
5. 最后用一个最小 eBPF tracepoint 程序按 `net/queue_poll` 附着，核对 `format` 后读取记录；该程序及其统计输出不作为网络栈方案的一部分。

### 4.2 必须留下的证据

低层测试应验证一次成功 `claim()` 对应一次报告，提前返回不重报，`Failed` 不谎报工作量，以及启停不改变 `GroupPollOutcome`。StarryOS 运行测试应验证事件发现、字段布局、至少一次真实触发与关闭后不再接收记录；不要求在 QEMU 中人为制造全部四类结果。eBPF 冒烟测试只证明 `load → attach → enable → read` 连通。沿用项目的 `cargo xtask` 验证入口，避免为每个字段和结果码建立重复的高成本系统用例。

热路径另需在相同配置与负载下比较未安装观察端口、已安装但事件关闭、事件开启三种状态的 CPU、吞吐和尾延迟；QEMU 证明装配与语义，不能替代目标板卡上的成本结论。若 IRQ 或跨 CPU 事件随后纳入，补充确定性的停止、并发启停和在途回调释放证明。高风险的共享回调边界应先由网络运行时与 Starry tracepoint 维护者审查，再合入实现。

### 4.3 回滚与完成判定

首个事件应能独立回滚：移除 Starry 事件适配和 `ax-net` 观察调用，不影响已交付的 `NetQueueIdentity`、`NetQueueStats`、`net_queue` 快照、SPSC 或 DMA 所有权。若为了生成记录必须修改网络结果、保存逐帧状态或引入第二份启用真相，应停止该事件并重新评审，而不是把副作用算作观测基础设施。

交付完成的标志是 `net:queue_poll` 有稳定文档化的触发与字段契约，真实队列操作产生正确记录，关闭态不改变网络行为且成本经过比较，tracefs 消费可用，并有一次最小 eBPF 附着验证。`queue_rearm` 及其余候选未通过各自准入评审时应继续标为候选，不因首个事件完成而宣称整个候选清单完成。
