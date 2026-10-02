# 交接：网络事件面（`feat/net-observe`）下一阶段

总结时间：2026-10-02。本文是个人文档，不被项目追踪；项目内的权威约定见 `docs/design/ax-tracepoint.md` 与 `docs/docs/architecture/net/events.md`。

## 1. 一句话状态

第一批（队列身份与计数出口）与第二批的全部执行器侧事件（`net:queue_poll_round` + `queue_rearm` + `queue_backpressure` + `tx_submit` + `rx_publish`，含端口泛化与门控 sink 注册表）已实现、已提交、已按第 4 节的证据验证；**框架侧不需要再改，剩余候选只有协议侧 `net:proto_poll`（收窄形态）与被 IRQ 安全证明挡住的 `net:queue_irq`**。协议侧另外三件（`tx_queue`/`rx_consume`/`route_drop`）经调查与对抗性复核后暂缓，理由见方案 §10。

## 2. 工作线坐标

- 工作树：`/home/asta/tgoskits/wt-feat-net-observe`；分支 `feat/net-observe`；fork `Antareske/tgoskits`；基线是上游 `dev` 的 `d93f125cc`。
- 本分支相对 `dev` 的 6 个提交（作者 `Antareske <noreply>`；前 5 个已推送，`2558509ab` **未推送**）：

| 提交 | 内容 |
| --- | --- |
| `2a24ea55f` | `feat(net): expose per-queue identity, counters and poll rounds`——队列身份（发现序索引/group ID/owner CPU）、`NetQueueSnapshot`/`net_queue_snapshots()`、接口绑定、丢弃双计数、`/sys/kernel/debug/net_queue`、观察端口本体与 `poll()` 一次一报、`qemu/system/net-queue` 用例 |
| `55a40de7e` | `refactor(starry-kernel): route runtime gate updates through registered sinks`——事件模块自行注册门控镜像，注册表统一回调（取代逐模块 `ptr::eq` 硬编码） |
| `4e99a24c6` | `feat(net): report queue poll rounds as net:queue_poll_round`——事件定义、适配层、系统用例 `qemu/system/net-events`、eBPF 冒烟 app、`events.md` 与框架约定表 |
| `326447169` | `refactor(tracepoint): state the gate sink contract and complete arity exemptions`——sink 契约文本、`register`/`publish` 锁纪律统一、重复注册断言、`gate` 模块放宽为 `pub(crate)`、宏对会超阈值的生成函数补齐 arity 豁免 |
| `beba1c0aa` | `fix(net): tighten the queue poll event's scope, proofs and docs`——`Blocked` 文档措辞与判别值 const 断言、端口单测补强（More/Blocked 覆盖、失败轮次工作量改为显式常量、开关比较改为非平凡轮次）、eBPF app 通过条件收敛为 `total >= 1`、SAFETY 注释改事实、系统用例加关闭沉降与放宽流量重试、`events.md` 三态成本与 Linux 对照、`api.md` 小节顺序 |
| `2558509ab` | `feat(net): report rearm, backpressure and frame handoffs as net:* events`——端口泛化为 `ObservationPort<T>`，新增 `net:queue_rearm`/`queue_backpressure`/`tx_submit`/`rx_publish` 四个事件的报告点、适配、gate sink 注册、端口单测与系统用例扩展，`events.md`/`api.md`/`devices.md`/`testing.md` 同步 |
| `829f4173e` | `feat(net): report protocol executor yields as net:proto_yield`——协议执行器预算耗尽让出报成事件；`ProtocolPollBudget::consume()` 改为返回让出原因，`observe.rs` 上移到 crate 根，系统用例扩展为六事件、四个流量驱动 |
| `b68e8bc50` | `refactor(tracepoint): keep only the arity exemption the macro needs`——按用户决定把 `too_many_arguments` 豁免收敛为只保留真正越界的 `trace_default_<name>`，另两处删除，`docs/design/ax-tracepoint.md` arity 行同步 |
| `986d40b9f` | `fix(net): align the net:* event contract with its evidence`——第三轮 OCR 的 3 项 blocker + 7 项 should fix：`QueueBackpressureReport.reason` 类型化并统一分类入口、补 RX 补投重试/LinkDown/永久拒绝/保留帧发布用例、rearm 断言改全向量、系统用例逐事件等待与判据加固、`events.md`/`testing.md`/`api.md`/`devices.md` 口径修正 |

后两支是第二轮 OCR 审查（第 5 节）的 should fix 修复。**按指示，修复后没有复跑测试与 QEMU，也没有重跑 OCR**；静态检查结果见第 5 节。

- 个人文档（未追踪）：
  - `www/starry-network-observability-plan.md`：方案（§3.2 候选事件表、§4 准入、§5 分批、§6 验收、§8 落地记录）。
  - `www/starry-network-events-explained.md`：面向理解的事件/端口说明。
  - `www/net-observe-*.log`：各轮验证日志（见第 4 节）。

## 3. 下一阶段要遵守的框架约定（已固化，勿另起炉灶）

权威表格在 `docs/design/ax-tracepoint.md` 的「新增事件：约定与责任划分」，要点：

1. **触发点在别的 crate 时**不要在追踪层定义事件：由事实所有者（如 `ax-net`）暴露窄观察端口（一个函数指针槽 + 一个已发布标志，安装一次、幂等、替换即断言、不卸载），宿主的事件适配模块安装回调并调用生成的 `trace_<name>()`。现成范例：`net/ax-net/src/observe.rs` + `os/StarryOS/kernel/src/tracepoint/net.rs`。
2. **门控**：事件模块在 `install()` 里 `crate::tracepoint::gate::register(&__<event>, <发布函数>)`，注册表在回调集合变化时统一 `publish`。sink 必须只发布一个原子状态：不阻塞、不分配、不 panic、不得回调追踪层（`update`/`register`/`publish`）——它运行在注册表 update 的临界区内（本模块的锁已释放）。重复注册同一事件会断言失败。
3. **字段**：只用固定宽度整数与定长数组；宽度转换在适配层做，**以饱和代替截断**（见 `net.rs` 的 `field()`）；结果码若是对外契约（如 `QueuePollOutcome` 的 0..3），用 `#[repr(u32)]` + const 断言钉住。
4. **子系统的 `(system, name)` 唯一**：跨分支合入同名事件前先对账（目前只是约定，`init_events` 的 `BTreeMap::insert` 会静默覆盖）。
5. **arity**：生成函数的形参随字段数增长，宏只对**会超过 lint 阈值**的生成函数承担 `too_many_arguments` 豁免；加字段时若新增超阈值函数，要在 `basic_macro.rs` 同步加豁免。
6. **验收三步**：低层单测证明触发边界（一次触发恰好一条、提前返回不重复、失败不谎报工作量）→ 系统用例证明事件可发现、`format` 与契约一致、真实触发、关闭后无记录 → 可选的 eBPF 程序只证明 `load → attach → enable → read`（不得把事件语义写进通过条件）。

## 4. 验证证据（本轮，修复前）

| 项 | 结论 | 日志 |
| --- | --- | --- |
| `qemu/system/net-events` | 四架构全通过（x86_64/aarch64/riscv64/loongarch64） | `www/net-observe-checks-events3.log`、`www/net-observe-checks-events3-arches.log` |
| `qemu/system/net-queue`（回归） | 通过 | `www/net-observe-checks-events3.log` |
| `apps/starry/ebpf/net_queue_poll` | 四架构通过（5/5/7/5 条记录） | `www/net-observe-checks-app-arches.log`、`www/net-observe-checks-app-loong.log`、`www/net-observe-checks-app-final.log` |
| `clippy --since dev`（ax-net + ax-tracepoint + starry-kernel，140 项） | 全通过 | `www/net-observe-clippy-final2.log` |
| `test --since dev`（15 个受影响包的标准库测试） | 全通过 | `www/net-observe-test-final.log` |

执行器侧四事件（`2558509ab`，本轮，修复后完整复跑）：

| 项 | 结论 | 日志 |
| --- | --- | --- |
| `qemu/system/net-events` | 四架构全通过；真实流量下读到 `queue_poll_round(budget=256 work_units=3 outcome=0)`、`tx_submit(frame_len=60)`、`rx_publish(frame_len=64)` | `www/net-observe-phase2-qemu2.log` |
| `qemu/system/net-queue`（回归） | x86_64 通过 | `www/net-observe-phase2-qemu2.log` |
| `apps/starry/ebpf/net_queue_poll` | x86_64 通过（5 条记录、`inconsistent=0`）；补跑它是因为 `net.rs` 的 `install()` 改为注册五个 gate sink，而 perf/BPF 附着是发布 gate 的另一条通路，`net-events` 用例只覆盖 tracefs `enable` 那条 | `www/net-observe-phase2-app.log` |
| `clippy --since dev` | 5 个包、140 项检查全通过 | `www/net-observe-phase2-clippy.log` |
| `test --since dev` | 15 个包全通过（`ax-net` 144 项单元测试） | `www/net-observe-phase2-test.log` |

`net:proto_yield`（`829f4173e`，本轮，完整复跑）：

| 项 | 结论 | 日志 |
| --- | --- | --- |
| `qemu/system/net-events` | 四架构全通过；x86_64/aarch64/riscv64 读到真实 `proto_yield` 记录（`reason=1 work_pending=1` / `reason=1 work_pending=0`），关闭后零记录 | `www/net-observe-phase3-qemu.log` |
| `qemu/system/net-queue`（回归） | x86_64 通过 | 同上 |
| `apps/starry/ebpf/net_queue_poll` | x86_64 通过（8 条记录、`inconsistent=0`），证明 perf/BPF 附着通路在新增第六个 gate sink 后仍工作 | 同上 |
| `clippy --package ax-net` / `--package starry-kernel` | 9 项 / 80 项全通过 | `www/net-observe-phase3-clippy.log` |
| `test --since dev` | 15 个包全通过（含 `poll_runtime` 三项预算分类测试） | `www/net-observe-phase3-test.log` |

第三轮 OCR 修复（`986d40b9f`，修复后完整复跑）：

| 项 | 结论 | 日志 |
| --- | --- | --- |
| `qemu/system/net-events` | 四架构全通过（新的逐事件等待判据；三架构回显 `proto_yield reason=1 work_pending=1`，关闭态零记录） | `www/net-observe-r3fix-qemu.log` |
| `qemu/system/net-queue`（回归） | x86_64 通过 | 同上 |
| `apps/starry/ebpf/net_queue_poll` | x86_64 通过（7 条记录、`inconsistent=0`） | 同上 |
| `clippy --package ax-net` / `--package starry-kernel` | 9 项 / 80 项全通过 | `www/net-observe-r3fix-clippy.log` |
| `test --since dev` | 15 个包全通过（`ax-net` 149 项，含四条新增端口用例） | `www/net-observe-r3fix-test.log` |

PR #2559 的 CI 修复（`d6a887460`）：

| 项 | 结论 | 日志 |
| --- | --- | --- |
| 根因 | `pseudofs/debug.rs` 的 `net_queue_tests` 只标 `#[cfg(test)]`，ktest（`--cfg axtest`）下被编译但 `#[test]` 函数无人收集 → 死代码 + 未使用导入，`-D warnings` 报错；改为 `#[cfg(all(test, not(axtest)))]` | — |
| `cargo xtask ktest qemu -p starry-kernel --arch x86_64` | 通过（212+ 用例） | `www/net-observe-ktest.log` |
| `test --since dev`（修复后） | 15 包全过，宿主仍运行 `net_queue_*` 用例 | `www/net-observe-r3fix2-test.log` |
| `apps/starry/ebpf/net_queue_poll` 四架构 | 全过（5/5/7/4 条记录，`inconsistent=0`），用于支持 README 支持表 | `www/net-observe-ebpf-arches.log` |

推到 `d6a887460` 后 PR #2559 的 CI 23/23 全绿（含四个 Starry QEMU 与 AxVisor 三项；上一次的 `AxVisor / QEMU loongarch64` 失败为偶发，未复现）。机器人 `mai-team-app[bot]` 的 CHANGES_REQUESTED 仍停在 `986d40b9f`，未重审新提交；按用户要求未在 PR 上回复。

调试期的诊断结论（避免重复踩坑）：Starry 的 AF_PACKET 是合成实现（`os/StarryOS/kernel/src/file/packet.rs` 只为建模网关的 ARP 请求造回应，帧不上线），**不能用它驱动真实队列**；loopback 也不经物理队列；驱动事件要用发往 QEMU 用户态网关的真实数据报。tracefs `trace` 的渲染是 `name(field=...)`（不是 Linux `trace_pipe` 的 `name: field=...`）。

## 5. 已修与未修

- 第二轮 OCR 审查（`.ocr/sessions/2026-10-02-feat-net-observe/rounds/round-2/`，结论 APPROVE、0 blocker、12 should fix、5 suggestion）的 **12 项 should fix 已全部修复**，落在 `326447169`、`beba1c0aa`；其中「api.md 小节错位」与「eBPF 注释声称的 format 校验」两处是提交引入的硬伤。
- **修复后未复跑**：QEMU 用例、`xtask test`、OCR。`cargo fmt` 与本地编译（C 用例四架构、eBPF app 的 x86_64 musl 构建）已过；`clippy --package ax-net` 复跑通过（该轮先抓出测试里一处多余的 `mut`，已修并 amend 进 `beba1c0aa`）；`clippy --package starry-kernel` 与 `--package ax-tracepoint` 复跑同样通过（`www/net-observe-clippy-fix2.log`）。
- **未修的 suggestion 级发现**（下一阶段可顺手处理，或有意识搁置）：
  1. `(system, name)` 重名会在 tracefs 静默覆盖（`init_events` 的 `BTreeMap::insert`）；
  2. perf/BPF 附着路径没有 capability 检查（既有缺口，非本改动引入）——建议至少在准入表写明「消费者需特权」；
  3. 适配层 `on_queue_poll` 的 6 个同型位置参数在四架构环境下取值全为 0，不可判别（覆盖边界，建议写进 `events.md`/`testing.md`）；
  4. 全局端口单测的卫生（断言失败时 gate 不复位、tag 占用靠约定）；
  5. 系统用例与 eBPF app 各有一份流量驱动常量（已加交叉注释，未抽公共实现）。
- **明确的未完成项**：端口开销的对照构建（「未安装端口」vs「已安装未启用」）按约定后置到合入后的板卡测量；事件契约中「失败轮次不计入轮预算记账」目前只在 `executor/mod.rs` 注释里，尚未写进 `events.md`。

## 6. 本轮已推进的部分（2026-10-02 晚）

执行器侧四个事件（`net:queue_rearm`、`net:queue_backpressure`、`net:tx_submit`、`net:rx_publish`）与端口泛化已实现：
`observe.rs` 的端口泛化为 `ObservationPort<T>`（五个事件各一个实例，逐事件 `install_*`/`publish_*_gate` 出口不变），
报告点分别在 rearm 状态转移（`state.rs` 竞态 + `executor/mod.rs` 的其余三种结局）、TX 提交与 RX 补投的设备拒绝、
驱动接纳帧、协议侧 RX 环发布成功；文档（`events.md` §2 五节、`api.md` §3.2、`devices.md` §12、`testing.md` §3.7）
与系统用例（五个事件的发现/格式/关闭态，流量驱动三个还要证明真实记录）同步更新。设计见方案 §9。

`net:proto_yield`（协议执行器让出）已在同一分支实施：触发点是 `poll_protocol_until_idle()` 的预算耗尽分支
（锁外），字段 `owner_cpu`/`reason`/`work_pending`，设计见方案 §9.7（含命名理由与频率口径）；`observe.rs`
为此上移到 crate 根，`ProtocolPollBudget::consume()` 由 `bool` 改为 `Option<ProtoYieldReason>`。

协议侧其余候选的调查结论见方案 §10：原 `proto_poll` 候选以收窄形态落地并改名 `proto_yield`，
`tx_queue`/`rx_consume`/`route_drop` 暂缓——事实只在协议锁内（`SERVICE` + `SOCKET_SET.inner` 跨整个 `Service::poll`），锁外报告要新增跨锁暂存；`route_drop` 还至少四处丢弃点完全没有计数；
复核还修正了一处夸大论断（锁序文档只规定获取顺序与持锁禁 `wake()`，并未禁止持锁运行消费者）。

## 7. 下一阶段的候选与准入

方案 `www/starry-network-observability-plan.md` §3.2 / §5.2 里第二批剩余候选（第一个已完成，其余待做）：

| 候选 | 触发边界 | Linux 对照 | 实施前必须先固定 |
| --- | --- | --- | --- |
| `net:queue_irq` | `PollGroupState::schedule_irq()` 观察到中断 | Linux IRQ 事件与 `napi:napi_poll` | 硬 IRQ 上下文的消费者可执行性（准入表的硬性前提：解释器/helper/映射/错误路径都不睡眠、不分配），否则改用延迟环 + 丢弃计数 |
| `net:queue_rearm` | rearm 发现已有工作等待或竞态分支 | 无逐字段等价事件 | 只在真实异常转移处发出，不重复既有 `rearm_race` 计数 |
| `net:queue_backpressure` | TX 提交返回可重试/链路不可用 | `net:net_dev_xmit`、`napi:dql_stall_detected` | 报告哪一层（设备错误层 `NetDeviceError::Again` 还是驱动返回层 `NetError::Retry`），不假称 `NETDEV_TX_BUSY` 语义 |
| `net:route_drop` | 帧在分发路径被最终拒绝 | `skb:kfree_skb` 的 drop reason | 唯一触发点与稳定原因分类；`Again` 不算丢弃 |
| `net:proto_poll` | `poll_protocol_until_idle()` 一轮结束 | 无（Starry 自有对象） | 需要时再加，不记录专用 duration |
| （第三批）`net:tx_queue`/`tx_submit`/`rx_publish`/`rx_consume` | 协议交付、驱动提交、队列发布、协议消费 | `net:net_dev_queue`/`net_dev_start_xmit`/`net:netif_rx`/`net:netif_receive_skb`（令牌回收到 `skb:consume_skb` 是另一处边界） | 只有第一批+计数仍无法区分队列边界故障时才加；不承诺逐帧配对、不加逐帧时间戳。`tx_submit`/`rx_publish` 已在执行器侧实现，`tx_queue`/`rx_consume` 按方案 §10 暂缓 |

加事件的机械步骤（照 `net:queue_poll_round` 抄即可）：观察端口（若事实在 `ax-net`）→ 事件定义 + 适配 + `install()` 注册 gate sink → `tracepoint_init()` 里加一行 `x::install()`（若新模块）→ 低层单测（触发边界）→ `qemu/system/<case>` 系统用例 → 可选 eBPF app → `events.md` 事件契约 + `api.md`/`devices.md`/`testing.md` 同步。

## 8. 关键文件索引

| 路径 | 作用 |
| --- | --- |
| `docs/design/ax-tracepoint.md` | 追踪层设计 + 「新增事件：约定与责任划分」权威表 |
| `docs/docs/architecture/net/events.md` | 网络事件契约（端口语义、字段表、三态成本、准入、验收） |
| `net/ax-net/src/observe.rs` | 六个事件的观察端口、报告类型与结果码（2026-10-02 由 `queue_runtime/` 上移到 crate 根） |
| `net/ax-net/src/queue_runtime/executor/mod.rs` | `poll()` 报告点、`GroupPollOutcome`、预算记账 |
| `net/ax-net/src/queue_runtime/executor/queue_tests.rs` | 端口单测（含 `port_identity`/`begin_poll_capture` 脚手架） |
| `os/StarryOS/kernel/src/tracepoint/gate.rs` | 门控 sink 注册表（契约、锁纪律、去重断言） |
| `os/StarryOS/kernel/src/tracepoint/net.rs` | `net:queue_poll_round` 事件定义与适配 |
| `test-suit/starryos/qemu/system/net-events/src/main.c` | 系统用例（真实流量、正负两相判据） |
| `apps/starry/ebpf/net_queue_poll/` | eBPF 冒烟 app（附着链路） |
| `.ocr/sessions/2026-10-02-feat-net-observe/rounds/round-2/final.md` | 第二轮审查结论与 12 项 should fix 原文 |

## 9. 未做的动作

推送与开 PR 需用户批准。分支 9 个提交已全部推送；PR 已按用户要求在 2026-10-02 创建：
`rcore-os/tgoskits#2559`（标题 `feat(ax-net): expose per-queue state and net:* tracepoint events`，基线 `dev`）。
PR 正文见 `www/starry-network-events-pr.md`（用户定稿版）。
