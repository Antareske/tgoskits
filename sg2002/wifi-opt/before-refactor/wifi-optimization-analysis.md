# SG2002 StarryOS WiFi 驱动优化空间与异步优化分析

**日期**：2026-07-24

## 目录

- [1. 当前架构分析](#1-当前架构分析)
- [2. 进一步优化空间](#2-进一步优化空间)
- [3. 异步优化：价值与可行性](#3-异步优化价值与可行性)
- [4. 优化路线建议](#4-优化路线建议)

---

## 1. 当前架构分析

### 1.1 任务模型

WiFi 驱动通过 `WifiRuntime::spawn_poll_task` 创建**5 个独立内核线程**：

| 线程 | 名称 | 唤醒源 | 职责 |
|------|------|--------|------|
| TX | `wifi-tx` | `wake_pollset` (数据入队) | CMD 发送 + DATA 批量发送 |
| TX-kick | `wifi-tx-kick` | 自身 sleep_ms(10) 自唤醒 | 兜底：丢失唤醒后 10ms 内救醒 TX |
| RX | `wifi-rx` | `irq_waker` (SDIO ISR → AtomicWaker) | SDIO FIFO 读取 + 帧分发 |
| RX-kick | `wifi-rx-kick` | 自身 sleep_ms(10) 自唤醒 | 兜底：异步入站帧（EAPOL M1 等）被及时捞出 |
| AP | `wifi-ap` | `assoc_pollset` (RX 线程入队关联请求) | STA 注册/注销 + Assoc Response |

每线程 = 一个 `ax_task` 线程 + 独占栈。TX 和 RX 共享一把 SDIO 锁 (`Mutex<dyn SdioHost>`)。

### 1.2 唤醒机制

```
数据面 TX: AicTxQueue::submit → enqueue_data_frame → wake_pollset.wake()
                                                      → wifi-tx 线程被唤醒
                                                      → tx_process → process_data_tx
                                                      → send_single_data_frame
                                                      → write_fifo (同步 PIO)

数据面 RX: ISR(IRQ#38) → sdio1_irq_handler
          → mask_card_irq + irq_pending.store(true) + irq_waker.wake()
          → wifi-rx 线程被唤醒
          → process_rx_frames → drain_func → read_fifo_data (同步 PIO)

控制面:   send_cmd → pending_flag + wake_pollset.wake()
          → wifi-tx 线程 → process_cmd_tx → write_fifo (同步 PIO)
          → block_until(timeout, rsp_pollset) ← 阻塞等 CFM
          → RX 线程收到 CFM → rsp_pollset.wake() → block_until 返回
```

### 1.3 关键瓶颈

**A. PollSet 无 sticky 位 → 丢失唤醒 → 10ms kicker**

`PollSet` 的设计是"先做事再注册 waker"：如果 `wake_pollset.wake()` 在 `register(cx.waker())` 之前触发，本次唤醒永久丢失。代码注释（`tx.rs:88`）明确指出了这个竞态：

```
// PollSet 无 sticky 位,若 wake_pollset.wake() 触发时 TX 的 waker 恰不在 set 里
// (例如 TX 正在 tx_process 内的 yield 点),这次唤醒会永久丢失
```

10ms kicker 是兜底方案，但它引入了 **10ms 的延迟地板** 和周期性 CPU 唤醒开销。

**B. SDIO 操作完全同步 → 持锁阻塞**

每个 `write_fifo` / `read_fifo` 调用内部走完整 PIO 流程：

```
write_fifo → cmd53_write_fixed → cmd53_xfer → wait_cmd_idle → send_cmd → wait_cmd_complete
          → pio_write → wait_buffer_write_ready (每块) → wait_transfer_complete
```

整个调用链持锁、不返回，直到硬件完成传输。TX 和 RX 无法在 SDIO 层面交错。

**C. CMD 阻塞提交 → 数据面暂停**

`send_cmd` 使用 `block_until(timeout, rsp_pollset)`——这会阻塞调用线程（CMD 走 TX 线程），最长可能等待数秒直到固件回 CFM。在此期间 TX 线程无法发送数据帧。

**D. PIO 模式下 CPU 全程参与传输**

每个字节的 FIFO 写入都需要 CPU 轮询 `BUF_WR_READY`。即使 Part 1 修复把忙等从 50ms 降到 ~200µs，这 ~200µs 内 CPU 仍然 100% 被占用。

---

## 2. 进一步优化空间

将报告中的 6 项优化设为"已完成"前提（Part 1 busy-wait + Part 2 五项），以下分析**在此基础之上**的进一步优化空间。

### 2.1 吞吐类优化

#### DMA (ADMA2) — 优先级：★★★★★

| 维度 | 说明 |
|------|------|
| **原理** | SDHCI 硬件支持 ADMA2（CAPABILITIES bit19=1）。DMA 引擎按描述符表自动执行 CMD53 传输，无需 CPU 逐字节灌 FIFO。 |
| **收益** | 报告的剩余差距估占 ~30%。DMA 下每帧 CPU 参与时间从 ~200µs 降至 ~10µs（提交描述符），且 CPU 可在传输期间处理其他任务。 |
| **风险** | (1) RISC-V 的 cache coherency：需要确保 DMA 描述符和数据缓冲区在 DMA 访问前已刷出 DCache；(2) ADMA2 描述符格式兼容性：需验证 CV1800 的 SDHCI 实现了标准 ADMA2；(3) 小帧场景（管理帧、ARP）DMA 设置开销可能超过 PIO，需要 PIO/DMA 自适应阈值。 |
| **难度** | 高（需 1-2 周） |
| **依赖** | 无（独立于其他优化） |

#### 多帧 CMD53 拼包 — 优先级：★★★★

| 维度 | 说明 |
|------|------|
| **原理** | 当前每帧一次 CMD53 写。改为将多帧拼成一个 CMD53 块传输（vendor `aicwf_sdio_aggr` 的模式），减少 CMD53 命令/状态开销和 `wait_transfer_complete` 调用次数。 |
| **收益** | 报告的剩余差距估占 ~10%。实测聚合 4-8 帧可减少 ~60-80% 的 SDIO 事务数。 |
| **风险** | (1) 拼包增加延迟：需设置最大攒包等待时间（如 1ms）或攒包数量上限；(2) 固件侧需支持多帧解包——需验证固件接口兼容性；(3) 与流控交互：拼包后单次 CMD53 更大，更容易触发 flow_control 反压。 |
| **难度** | 中 |
| **依赖** | 需先完成流控空转修复（Part 2.4），否则拼包加剧 fc 阻塞 |

#### HE (802.11ax) 数据通路修复 — 优先级：★★★★★

| 维度 | 说明 |
|------|------|
| **原理** | 当前 HE 协商成功（IE FF:23）但 `ampdu=0`。修复固件消息格式或配置序列使 HE 数据通路通。PHY 速率从 HT-MCS7 (65M) 提升到 HE-MCS9 (115M)，这是 ~60% gap 的主体。 |
| **收益** | 理论上行从 ~13.7M → ~20M+（1.77× PHY 增益，扣除协议开销）。这是追平 Linux 33M 的最大单一变量。 |
| **风险** | (1) 三次验证均未解决，根因可能在固件内部而非 host 侧；(2) 可能需要逆向 vendor 驱动中未文档化的 HE 配置序列；(3) 射频校准差异可能导致 HE-MCS 实际达不到标称速率。 |
| **难度** | 高（需固件逆向） |
| **依赖** | 需要 eBPF tracepoint 监测系统来快速迭代验证 |

### 2.2 延迟类优化

#### 消除周期性 kicker → 纯事件驱动 — 优先级：★★★★

| 维度 | 说明 |
|------|------|
| **原理** | 将 PollSet 改造为 sticky 模式（记住"曾经被 wake 过"），或重构 poll 循环为"先 register waker 再消费工作"。消除 10ms kicker 后 ping RTT 应从当前的 ~10ms 众数降到 ~1-3ms（本地 WiFi 预期值）。 |
| **收益** | 报告指出 kicker 10ms→1ms 提升 +20% 吞吐。彻底消除 kicker 可进一步降低延迟、减少 CPU 周期性唤醒开销，且让管理帧握手（Auth/Assoc/EAPOL）不再有 10ms 的随机延迟抖动。 |
| **风险** | (1) 代码注释指出问题根因是 PollSet 无 sticky + 队头阻塞（pending CMD 阻止 DATA 处理），仅改 sticky 不够——CMD 和 DATA 的 wake 需要独立；(2) 需要充分的竞态测试覆盖（低负载+高负载+边界）。 |
| **难度** | 低~中 |
| **依赖** | 无 |

#### SDIO 锁拆分 → TX/RX 并行 — 优先级：★★★

| 维度 | 说明 |
|------|------|
| **原理** | 当前 `Arc<Mutex<dyn SdioHost>>` 是单一粗粒度锁。SDHCI 规范中 CMD 线和 DAT 线可独立操作——可在 TX 写 FIFO 的同时 RX 读 FIFO（但需要硬件支持 full-duplex）。对于 half-duplex 硬件，至少可用读写锁（RWLock）让多读者共享。 |
| **收益** | 实测 TX 满载时 RX 6 秒仅收 3 帧（报告记录）。锁拆分后 TX 和 RX 可以在 PIO 忙等的间隙交错推进。 |
| **风险** | (1) CV1800 SDHCI 是否支持全双工未验证；(2) SDIO 规范对 CMD53 交错有严格限制；(3) 当前 PIO 模式依赖轮询寄存器，交错操作可能导致状态机混乱。 |
| **难度** | 中~高（需要硬件验证） |
| **依赖** | 建议 DMA 化后再做（DMA 自然解耦了 CPU 和总线） |

### 2.3 稳定性类优化

#### 固件异常恢复 — 优先级：★★★

| 维度 | 说明 |
|------|------|
| **原理** | `poll_int_status` 的 Phase 2 超时（100,000 次 yield）触发 DAT 线复位，但未尝试完整的总线/芯片复位。在固件 wedge 场景（如 HE 数据通路断、流控死锁），当前只能靠用户重启整板。 |
| **收益** | 提高长时运行可靠性，尤其对机器人等场景（需无人值守运行数小时）。 |
| **风险** | (1) 复位序列需从零实现（固件重加载 + LMAC 重配置 + VIF 重建）；(2) 上层连接状态需感知复位事件并触发重连。 |
| **难度** | 中 |
| **依赖** | 需要 eBPF 监测来定义"异常"的判定阈值 |

#### WPA2 加密的 SoftAP — 优先级：★★

| 维度 | 说明 |
|------|------|
| **原理** | 当前 SoftAP 仅支持 open 网络。补充 WPA2-PSK AP 模式的 4-way handshake（AP 侧作为 authenticator）。加密代码已有 `wpa2.rs`（supplicant 侧），authenticator 侧逻辑对称。 |
| **收益** | 满足实际部署的安全要求。 |
| **风险** | (1) 固件的 AP 模式 key install 接口未验证；(2) GTK 轮换需要定时器。 |
| **难度** | 中 |

---

## 3. 异步优化：价值与可行性

### 3.1 当前是否已经"异步"

从 Rust `Future` trait 的语义看，**当前架构已经是异步的**：

```
当前模型:                     Rust async/await 等价物:
spawn_poll_task(name, f)  ≈  tokio::spawn(async move { ... })
PollSet::register(waker)  ≈  cx.waker().clone()
PollSet::wake()           ≈  waker.wake()
Poll::Pending             ≈  Pending
Poll::Ready(())           ≈  Ready(())
block_until(timeout, f)   ≈  tokio::time::timeout(dur, fut)
```

`WifiRuntime` trait 本身就是对异步执行器的抽象，与具体 runtime 解耦。**将手写 poll 函数改写为 `async fn` 只是语法转换，不改变语义或性能。**

### 3.2 "异步优化"的实际含义

在这个驱动中，"异步"应理解为三个维度，而非语法糖：

#### 维度 1：硬件异步 — SDIO 操作非阻塞化

**含义**：CPU 发起 SDIO 传输后不必忙等，可以去做其他事，传输完成后由中断/事件通知。

**当前状态**：所有 SDIO 操作都是同步的。`write_fifo` 从头到尾持锁忙等，期间 CPU 无法做任何其他工作。

**实现路径**：

```
PIO (当前):                            DMA (异步):
CPU: submit → busy-poll → done        CPU: submit descriptor → yield (可做其他事)
SDIO: ------------transfer--→         SDIO: DMA engine ------------transfer--→
                                      完成: interrupt → wake task → 回收描述符
```

**价值**：
- 报告 ~30% 的 gap 归因于 PIO 开销。DMA 可释放这部分 CPU 时间。
- 在 TX 的 DMA 传输期间，CPU 可准备下一帧、处理 RX 数据、或执行网络栈。
- 单核场景下 DMA 的"CPU offload"价值最大，因为 CPU 是唯一稀缺资源。

**可行性**：
- 硬件明确支持 ADMA2（CAPABILITIES bit19=1）
- 需要实现 ADMA2 描述符管理、DMA 完成中断、缓存一致性处理
- **这是异步优化中最有价值、最可行的一项**

#### 维度 2：架构异步 — 操作流水线化

**含义**：解除不必要的前后依赖，让可以并行的操作交错执行。

**当前状态**：
- CMD 提交阻塞 TX 线程直到收到 CFM（可达数秒）
- TX 和 RX 串行竞争同一把 SDIO 锁
- 管理帧发送和数据帧发送使用同一队列，管理帧需要等数据帧排空

**实现路径**：

```
CMD 异步提交:
  当前: enqueue_cmd → wait_for_cfm → return   (同步，阻塞 TX 线程)
  改进: enqueue_cmd → return Promise-like handle
       TX 线程继续发数据帧
       收到 CFM 时 → wake_cmd_waiter          (异步，不阻塞)
```

**价值**：
- CMD 异步提交：在 WPA2 握手期间（EAPOL 有多次 CMD 往返），数据面可以继续工作，避免"连接时断网"
- 管理帧和数据帧分离：管理帧可插队发送（已实现 `push_front`），但仍有队头阻塞问题

**可行性**：
- CMD 异步提交：需将 `block_until` 替换为状态机（CMD_SENT → WAIT_CFM → DONE）
- 难度中等，但需仔细处理超时、错误、取消等边界
- **价值中等，建议在消除 kicker 后做**

#### 维度 3：调度异步 — 消除周期性轮询

**含义**：所有唤醒由真实事件驱动，不等 timer 到期。

**当前状态**：
- TX/RX 各有 10ms 周期性 kicker
- `check_data_flow_control` 中 50× 空转 yield（Part 2.4 应修复）
- CMD 响应等待使用 `block_until` + poll（内部也可能轮询）

**实现路径**：

```
消除 TX kicker:
  PollSet → StickyPollSet (记住曾 wake 过)
  + 将 register(waker) 提前到消费工作之前
  → 消除竞态窗口 → kicker 不再需要
```

已在 2.2 节详述，此处不重复。**这是低难度、高收益的异步改进。**

### 3.3 不推荐：`async/await` 语法糖层面的改造

将手写 poll 函数改写为 Rust `async fn` **不推荐**，原因：

1. **无性能收益**：手写 poll 和 async fn 编译后产生相同的状态机，只是语法不同。
2. **失去跨内核可移植性**：`async fn` 会耦合到特定 async runtime（tokio / embassy / custom executor），而当前 `WifiRuntime` trait 可以对接任何内核。
3. **调试复杂度上升**：`async fn` 的堆栈回溯在 `no_std` + 嵌入式中不如显式 poll 函数可读。
4. **依赖代价**：需要引入 `core::future::Future`、`core::pin::Pin`、异步块对 `alloc` 的依赖等，在当前 `no_std` + `alloc` 环境下增加复杂度。

### 3.4 异步优化价值-难度矩阵

```
收益
 高 │  ★ DMA (ADMA2)          ★ HE 数据通路
     │
     │  ★ 消除 kicker          ★ 多帧拼包
 中  │  ★ CMD 异步提交
     │                          ★ SDIO 锁拆分
     │  ★ WPA2 SoftAP           ★ 固件异常恢复
 低  │
     │
     └──────────────────────────────────────────
       低          中          高          难度
```

**异步相关**（绿色区域）的三项：
- **DMA**：唯一的"硬件异步"，高收益高难度，最值得投入
- **消除 kicker**："调度异步"，中等收益低难度，应优先完成
- **CMD 异步提交**："架构异步"，中等收益中等难度，DMA 之后做

---

## 4. 优化路线建议

### 阶段 0：基础修复（合入报告的 6 项优化）

```
0.1 poll_int_status 3ms 忙等         ─── Part 1 根因，无此前其他优化无意义
0.2 HT 结构体对齐                     ─── 解锁 A-MPDU 聚合
0.3 SDIO 50MHz + PHY delay           ─── 总线带宽翻倍
0.4 流控空转修复                      ─── 解除 50MHz 下的 fc 阻塞
0.5 TX/RX kicker 10ms → 1ms          ─── 延迟改善 +20% 吞吐
0.6 日志级别 Error                     ─── 消除同步 UART 阻塞

预期结果: 上行 0.2M → 13.7M, 下行 → 13.7M
```

### 阶段 1：可观测性基础设施

```
1.1 WiFi tracepoint 定义 + 埋点       ─── 后续所有优化的量化基础
1.2 eBPF wifi-monitor 工具            ─── 动态挂载, 无需重编译
```

### 阶段 2：异步架构改进

```
2.1 消除周期性 kicker                 ─── 低难度, 高收益
    基于 eBPF 数据验证"无丢失唤醒"后移除 kicker 线程
2.2 CMD 异步提交                      ─── 解除 CMD 阻塞对数据面的影响
```

### 阶段 3：吞吐突破

```
3.1 HE 数据通路修复                   ─── 最大单一增益 (~60% gap)
    利用 eBPF 逐事件计时快速迭代, 而非手工 log
3.2 DMA (ADMA2)                       ─── ~30% gap, 释放 CPU
3.3 多帧 CMD53 拼包                   ─── ~10% gap
```

### 阶段 4：完善

```
4.1 WPA2 SoftAP                       ─── 安全
4.2 固件异常恢复                       ─── 可靠性
4.3 SDIO 锁优化                        ─── 如果 DMA 后仍有争用
```

### 异步优化的核心结论

**异步优化有意义，但核心在"硬件的异步化（DMA）"和"调度的异步化（消除轮询）"，而非语法层面的 async/await。** 当前 `WifiRuntime` trait + poll task + PollSet 的架构已经实现了软件层面的异步抽象，且跨内核可移植。DMA 是唯一能让 CPU 真正从 SDIO 传输中解放出来的手段，是在单核 SG2002 上把 WiFi 吞吐推近 Linux 水平的关键路径。
