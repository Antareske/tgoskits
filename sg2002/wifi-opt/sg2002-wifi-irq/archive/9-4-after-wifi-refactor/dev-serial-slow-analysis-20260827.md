# dev 串口极慢分析（AP 启动后 CPU 节流，2026-08-27）

现象：纯 dev（wifi feature）上板 SG2002 后，`APM_START_CFM`（11.649s）之后串口逐字符输出，每条 busybox 命令 ~3.5s（newdev.log）。无 wifi feature 的 dev 无此现象（已实测确认）→ 归因 wifi 路径。

## 结论（两级机制叠加）

### 组件 1：新 NAPI 运行时执行器无 sleep 自旋（结构缺陷，已确认）

`net/ax-net/src/queue_runtime/executor.rs` 的 worker 主循环（468-511 行）：只有「无 group 处于 SCHEDULED 且无 pending wifi 请求」才 `control.notify.wait()`；仅当 `cpu_work >= CPU_ROUND_BUDGET(256)` 才 `yield_now()`。

因此只要 `rearm_and_check` 持续返回 `WorkPending`（`finish_idle` → `schedule_task` → 下一轮 `claim` → `poll` → `rearm`……），worker **既不 sleep 也不 yield，100% CPU 自旋**。旧模型的 RX/TX/AP 线程在 waker 上睡眠，同样的「条件卡住」只退化为睡眠/唤醒循环（CPU 大部分空闲）；新模型退化为 CPU 独占。

`WorkPending` 的持续来源（待板端确认二选一）：

- **a) 控制器 CARD_INT level 卡住**：V3（D80）的 `MISC_INT_STATUS` bit7（`SDIO_OTHER_INTERRUPT=0x80`）在 `read_block_count_with_retry` 中 3 次 `clear_v3_other_interrupt` 重试后仍置位 → drain 放弃（返回 (0,false)）→ 固件事件未被消费 → level 不撤 → `rearm_card_irq_and_check` 恒返回 pending → `rearm_and_check` 恒 `WorkPending`。APM_START 后固件开始周期性 CARD_INT 事件（beacon/indication），第一次触发该条件即卡住——与日志中 ~12.05s 后开始变慢吻合。
- b) `has_work()` 卡住（`pending_flag`/`pktcnt`/队列残留）——暂无线索，可能性较低。

### 组件 2：dev 的 sdhci PIO Phase 2 仍是「每圈 yield_now」（已确认）

`drivers/blk/sdhci-cv1800/src/lib.rs:134-159`：Phase 2 为 200_000 次「读状态 + `yield_now()`」协作等待。这正是 sg2002-wifi.md §3.3 记录的 50ms 时间片碰撞：一次 yield ≈ 48ms 调度周期往返。`e9e64d30c`（busy-wait 替代 yield）**不在 dev**（dev 的 sdhci 历史仅 crate 迁移 + 59fb32de6）。

### 主因判定

**组件 1 单独即可解释本次现象，组件 2 不是本次现象的主因。**

自旋循环的每轮迭代 = mask CMD52/MMIO + block_cnt CMD52 读 + rearm MMIO——这些短操作在 Phase 1（1000 次自旋，~100-200µs）内完成，**不落进 Phase 2、不触发 yield**。因此执行器完整持有自己的 50ms 时间片、从不主动让出，其它任务只能等 round-robin 切片边界——串口逐字符（每字符最多等 50ms）、每条命令 ~3.5s（约 70 次调度等待 × 50ms）。

组件 2（Phase-2 yield 的 ~48ms 过路费）在数据面传输（CMD53 数据相位等待）时才被触发，是独立的吞吐问题：旧文档实测「几乎每笔 TX 落进 Phase 2」发生在 CMD53 数据相位；本次自旋不含数据相位等待。仅当自旋间隙有固件事件到达、drain 真读到 FIFO 数据（CMD53 读）时才会间歇性命中组件 2。

## 与变基后本分支的关系

- 本分支已提交的 sdhci 中断驱动 Phase 2（XFER_COMPLETE + `block_timeout_until`）天然规避组件 2；
- 组件 1（执行器反自旋保护）是 dev 新运行时的通用缺陷，变基后的本分支同样存在，仍需处理。

## 修复方向（仅聚焦已确认机制）

1. 执行器反自旋：`rearm_race` 统计（executor.rs `stats.rearm_race`，已存在）连续增长超过阈值时强制 `yield_now()`/短暂 sleep，把自旋降级为限速轮询。
2. sdhci Phase 2：有界忙等替代每圈 yield（e9e64d30c 方向），或本分支的中断驱动 Phase 2。
3. V3 SW 中断卡住的根因（组件 1a）：板端探针确认 `SDIO_OTHER_INTERRUPT` 是否持续置位、drain give-up 路径是否高频命中；修法方向：give-up 前按厂商语义（clear 后重读一次，仍有数据则照常解析消费）处理。

## 板端验证探针

- 将 drain 中「`SDIO_OTHER_INTERRUPT persists after 3 retries, giving up`」的 trace 提为 info/warn，观察是否高频命中；
- 周期打印 `rearm_race` 统计，观察是否高速增长；
- 观察 `[SDHCI] poll_int mid-timeout`（Phase 2 命中 10 万圈的标志）是否出现。

## 板测结果（两轮探针，2026-08-27）

第一轮（aicprobe.log）：`controller CARD_INT still asserted` 以 ~16k/s 稳定增长（26 条存活，12288@13.16s → 1556480@110.9s）。注意：串口极慢导致日志大量丢失（存活率 ~7%），缺失计数不可采信——基于"0 计数"的推断全部撤回。

第二轮（aicprobe2.log，存活率提高）：

1. **rearm_race 自旋确认**：`[net-queue] rearm_race storm detected` 131072@15.97s → 589824@43.8s ≈ 16k/s——执行器确实在 finish_idle→rearm→WorkPending→schedule 路径上无 sleep 自旋。
2. **状态字节恒定 0x01**：`[wifi-rx] PROBE intstatus changed to 0x01 (count=1)` 出现在 7.516s（start_ap_open 的第一个命令交换期间），此后变化计数恒为 1——固件自此**持续**报告 MISC_INT_STATUS=0x01，同时 CARD_INT 电平持续断言。
3. **自旋起点**：count=32768@9.85s 外推 → 自旋始于 ~7.9-8.5s，即 APM_START_CFM（7.82s）+ 网络服务发布（7.87s）之后立即开始。
4. 0x01 在驱动语义里 = `Blocks(1)` = 每轮 512B 幻影 FIFO 读；排水对该恒定值无产出、CARD_INT 不撤。

**结论**：触发源是固件自启动中期（第一个命令交换）进入的某种"主机需服务"状态（0x01 + CARD_INT 持续），dev 的主机侧没有正确服务它。主假说：0x01 是固件的 host-wake/attention 请求（V3 睡眠握手），而 dev 的 RX 排水路径从不调用 `transport.wakeup()`（仅 TX 路径调用）→ 固件睡眠不醒、FIFO 读无意义、电平不撤。旧代码时序下未触发该状态（静态对比未见 wake 调用差异，差异在时序层面）。

## 第三轮（ap3.log，2026-08-27）：wakeup 实验——睡眠假说被排除（随后进入第四轮源码比对）

无改善。本轮的三个决定性事实：

1. `PROBE drain wakeup=true`——排水前唤醒握手成功，**芯片醒着**，不是睡眠问题。
2. `PROBE fifo read ok: len=512 head=[1c, 00, 11, 00, 6a, 00, 0d, 00]`——0x01 状态下 FIFO 读**成功**且返回真实形态数据（至少一次），不是读失败。
3. 自旋持续 ~15k/s（still asserted: 1@7.82s → 344064@31.3s），状态字节仍恒定 0x01。

**资产**：探针分支 `sg2002/wifi-napi-probe` @ `9afc8e210`（dev + 3 个探针提交）可继续复现；变基后分支 `sg2002/wifi-irq`（dev ba252ca67 + 10 个适配提交）；备份 `sg2002/wifi-irq-beforeNAPI`。

## 第四轮（2026-08-27）：厂商源码比对——根因定位

源码来源：LicheeRV-Nano-Build（osdrv/extdrv/wireless/aic8800/，含 aicwf_sdio.c hal_irqhandler D80 分支与 aicwf_txrxif.c；linux_5.10/drivers/mmc/host/，含 sdhci.c 与 sdhci-cv180x.c）。

### 1. CARD_INT 不 W1C 的疑点——已澄清，仍是标准协议

标准 Linux sdhci_irq（linux 5.10 sdhci.c:3592-3601）对 CARD_INT 的处理：mask 信号 + sdio_signal_irq，随后的 W1C 写**显式排除 CARD_INT**（`intmask &= ~(... | SDHCI_INT_CARD_INT)`），依赖状态位随卡线电平自清。本控制器（cv180x，NORM 寄存器即标准 SDHCI 偏移 0x30/0x34/0x38）在 vendor Linux 上以该流程正常工作——"ISR 从不 W1C CARD_INT"在本控制器上依然是正确协议，**不是重构引入的错误**。重构真正改变的：旧模型 unmask 后从不回读状态位（对位语义不敏感）；新模型 `rearm_and_check_card_irq` 回读该位并据此分支（电平语义成为负载假设）。因此 probe 中 rearm 持续 pending = DAT1 电平被固件真实拉低——固件确有未服务的事件，而非控制器锁存。

### 2. 根因：OTHER_INTERRUPT(0x80) 排水协议偏离 + 双消费者竞态

**（a）0x80 排水协议偏离（与 vendor hal_irqhandler D80 分支逐行比对）**

vendor 对 `intstatus & SDIO_OTHER_INTERRUPT != 0` 的处理：读 reg 0x01（INTR_PENDING）清 bit0（dev-to-host soft irq ack）写回，**然后仍按低 7 位计数读 FIFO**（`intmaskf2 = intstatus | 8`；func2 队列取 `intstatus & 0x07`、func1 队列取 `intstatus & 0x7F` 个块）——即 0x81 = ack 后读 1 个 block。

本驱动 `read_block_count_with_retry` 对 0x80 的处理：清 reg 0x01 bit0 后**重读状态，3 次后放弃返回 (0,false)，从不读 FIFO**。若固件在数据未被消费期间持续保持 0x80（或投递时置 0x80），则 FIFO 数据永远不被排水 → 固件持续断言 CARD_INT → 执行器自旋。这与全部探针计数吻合：

- 7.517s 唯一一次 FIFO 读 = **MM_SET_RF_CALIB_CFM**（msg_id=0x006A，dest=13=TASK_API），是唯一一次不带 0x80 的普通投递（状态字节记录 0x01）；
- 此后所有投递带 0x80（0x81），被 OTHER 放弃路径挡住——FIFO_READ_OK 恒 1、FIFO_READ_FAIL 恒 0、状态字节变化计数恒 1（0x81 的首次变化与随后值均被日志条件静默）；OTHER_INT_GIVEUP 的 warn 日志因串口丢失未存活（约 7% 存活率）。

**（b）双消费者竞态（新模型结构回归）**

旧模型（beforeNAPI）：单一 RX 线程独占排水；命令等待经 rsp_pollset 休眠，不做 SDIO。新模型：`send_cmd` 等待循环内调用 `progress_io`（tx_process + `process_rx_frames(64)`），与执行器 RX reclaim 路径**并发排水同一状态寄存器与 FIFO**（transport Mutex 仅序列化单笔 CMD52/CMD53，不序列化"读状态→读 FIFO"序列）。两次 drain 同时读到同一计数、其一消费帧、另一读空 FIFO（返回残留数据）——固件随后进入 0x80 注意态。竞态时机不定，解释 newdev（最后一笔 CFM 后卡死）与 ap3（第一笔 CFM 后卡死）的差异。

### 3. 修复方向（协议层，均有 vendor 依据）

- **A（必需）**：按 vendor 语义改写 OTHER 处理——ack 一次后剥掉 bit7，按低 7 位计数继续排水；计数为 0 则如常 break。删除 3 次重试放弃逻辑。落点：`drivers/net/aic8800/src/fdrv/thread/rx.rs` read_block_count_with_retry。
- **B（架构修复）**：消除双消费者——control 路径（send_cmd 等待）不再直接排水，回归 NAPI 契约"control 提交事务而非直接触碰 SDIO"；排水只由执行器 poll 步骤单点执行，control 等待 rsp_queue + 唤醒执行器。
- 原有反自旋保护（止血层）仍建议保留：level 卡住类故障在任何设备上都可能再发生。

### 4. 旧模型为何不触发

旧驱动同样"3 次重试放弃、不读 FIFO"，但（1）单消费者，无双 drain 竞态触发固件 0x80 注意态的机会；（2）旧模型的 RX 线程即使在 0x80 下放弃，下一次 CARD_INT 边缘仍会重新唤醒重试，固件状态有机会自愈；新模型的 rearm_and_check 状态机把"一次卡住"放大为永久 WorkPending 自旋。
