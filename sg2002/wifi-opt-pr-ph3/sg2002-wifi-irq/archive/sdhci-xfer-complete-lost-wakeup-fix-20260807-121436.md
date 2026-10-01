# SDHCI XFER_COMPLETE 丢唤醒窗口：考证与修复方案

**创作时间**：2026-08-07T12:14:36Z  
**来源**：PR review request change 反馈 `www/rc.md`  
**分支**：`sg2002/wifi-irq`

## 1. 问题判断

评审反馈正确。`poll_int_status` 中 XFER_COMPLETE 的 WaitQueue 等待存在丢失唤醒窗口，导致中断即时唤醒（微秒级）在实际完成路径中可退化为完整 10ms timeout，不符合本 PR 的中断即时唤醒目标。

## 2. 竞态分析

### 2.1 当前流程

`poll_int_status`（lib.rs:206-208）：

```
1. pre-check status → None（传输尚未完成）                         // line 188-191
2. unmask_xfer_complete_signal()                                 // line 207: 使能中断信号
      ↓  ←—— 竞态窗口：ISR 可在此触发
3. block_timeout()                                               // line 208: 入队 WaitQueue，阻塞
```

### 2.2 丢失唤醒的具体时序

1. Task 调用 `unmask_xfer_complete_signal()` → SIG_EN 中 XFER_COMPLETE 使能
2. **传输完成** → INT_STATUS XFER_COMPLETE 置位 → 电平触发的中断线断言
3. ISR `sdhci_irq_handler` 触发：
   - `rmw_norm_sig_en(base, 0, NORM_INT_XFER_COMPLETE)` — **mask 信号**
   - `SDHCI_PIO_WQ.notify_one_from_irq()` — **WaitQueue 为空，通知丢失**
4. Task 进入 `block_timeout` → `wait_timeout` → 入队 + 阻塞
5. XFER_COMPLETE 信号已被 mask，无新中断触发 → **Task 等满 10ms timeout**
6. 超时后 recheck 发现 sticky bit 已置位 → 成功返回

### 2.3 现有注释的缺陷

lib.rs:186-188：

> 真正的防护是 XFER_COMPLETE sticky bit + post-wake recheck

此注释在**正确性**层面成立（sticky bit 保证最终总能被消费），但在**延迟**层面失败：10ms timeout 退化否定了 PR 将 PIO 从纯轮询改为中断即时唤醒的设计目标。

### 2.4 单核假设不闭合此窗口

lib.rs:12-13 声明了单核串行化假设。但 `mmio_write`（unmask）返回后到 `block_timeout` 入队锁之间仅相隔几条指令，中断控制器可在任意指令边界触发 ISR。单核假设无法阻止此窗口。

## 3. 修复原理：利用 `wait_timeout_until` 的正确同步语义

### 3.1 WaitQueue 内部同步机制

`WaitQueue` 使用 `SpinNoIrq`（`BaseSpinLock<NoPreemptIrqSave>`），持锁期间禁止中断。`wait_timeout_until`（wait_queue.rs:148-179）的 condition 检查与 `blocked_resched`（入队+阻塞）在**同一 WQ 锁临界区内**执行：

```rust
let wq = self.queue.lock();    // 关中断
if condition() {               // 检查条件
    timeout = false;
    break;                     // 不阻塞，直接返回
}
rq.blocked_resched(wq);        // 入队 + 阻塞（仍持锁）
```

这意味着：只要 ISR 在 `wait_timeout_until` 获取 WQ 锁**之前**已将条件发布（写 atomic flag），condition check 就能观察到它。窗口被闭合，因为 condition check 与入队对 ISR 是原子的。

### 3.2 方案：AtomicBool 条件 + `wait_timeout_until`

新增 `AtomicBool XFER_COMPLETE_NOTIFIED`：

- **ISR**：检查到 XFER_COMPLETE 后，mask 信号，**先设置 flag=true，再通知 WQ**
- **Task**：进入 Phase 2 时清零 flag → unmask → re-check → 调用 `block_timeout_until(timeout, &flag)`，内部由 `wait_timeout_until` 以 flag 为条件

## 4. 修复方案

### 4.1 涉及文件

| 文件 | 改动摘要 |
|------|---------|
| `components/sdhci-cv1800/src/irq.rs` | 新增 `XFER_COMPLETE_NOTIFIED: AtomicBool`；ISR XFER_COMPLETE 路径在 mask 后设其为 `true`，再调用 pio_wake_callback |
| `components/sdhci-cv1800/src/runtime.rs` | `SdhciDelay` trait 新增 `block_timeout_until(&self, timeout_ms: u64, flag: &AtomicBool) -> bool`，含默认 polling fallback |
| `components/sdhci-cv1800/src/lib.rs` | `poll_int_status` XFER_COMPLETE 分支改为：清零 flag → unmask → re-check → `block_timeout_until(&XFER_COMPLETE_NOTIFIED)` |
| `os/arceos/modules/axruntime/src/wifi_glue.rs` | `ArceosDelay` 实现 `block_timeout_until`，委托给 `SDHCI_PIO_WQ.wait_timeout_until(…, \|\| flag.load(Acquire))` |

### 4.2 新流程（poll_int_status 的 XFER_COMPLETE 分支）

```
1. XFER_COMPLETE_NOTIFIED.store(false, Release)
2. pre-check status (poll_status_once) → None（未完成，继续）
3. unmask_xfer_complete_signal()            // 使能中断信号
4. re-check status → None                   // 关窄竞争窗口
5. block_timeout_until(timeout, &XFER_COMPLETE_NOTIFIED)
   └→ OS glue: wait_timeout_until(dur, || flag.load(Acquire))
        └→ lock WQ（关中断）
           if flag == true → 不阻塞，立即返回 false（未超时）
           blocked_resched(wq)             // 入队 + 阻塞（仍持锁）
6. 醒后 re-check status → 命中 → W1C 清除 → 返回 Ok(())
```

### 4.3 竞争场景覆盖验证

| 时序 | 结果 |
|------|------|
| ISR 在 step 4（re-check）**之前**触发 | re-check 发现 sticky bit → 立即返回，不经 WQ |
| ISR 在 step 4 **之后**、step 5 WQ 锁**之前**触发 | ISR 设置 `flag=true` + notify(空队列) → step 5 condition 命中 → 不阻塞 |
| ISR 在 WQ **持锁期间**触发 | ISR 无法触发（SpinNoIrq 关中断），没有任何丢失 |
| ISR 在**阻塞后**触发 | ISR 正常 notify → 任务即时唤醒 |

**所有路径均无丢唤醒。**

### 4.4 向后兼容

- `block_timeout_until` 提供默认实现（基于 `delay_ms(1)` 的轮询循环），不使用 WaitQueue 的 OS glue 自动退化为轮询，不会编译失败
- 非 XFER_COMPLETE bit 的等待路径完全不涉及此变更（仍使用 `delay_ms` 纯超时睡眠）
- 现有 `block_timeout` 接口保留不变

### 4.5 风格一致性

- `XFER_COMPLETE_NOTIFIED` 放置在 `irq.rs`，与现有的 `SDHCI_IRQ_COUNT`、`SDHCI_LAST_NORM`、`SDHCI_CARD_INT_COUNT` 等诊断原子量共处，命名和模式一致
- `block_timeout_until` 签名采用 `&AtomicBool`（而非泛型闭包），兼容 trait object（`&dyn SdhciDelay`），无需额外分配

## 5. 测试建议

### 5.1 回归测试

在 host 上构造"ISR 比任务先到"的确定性测试：

- 在 `unmask_xfer_complete_signal` 调用后插入可控延迟（模拟传输提前完成），验证 `block_timeout_until` 立即返回（timeout=false），而不是走满 10ms timeout
- 对比旧实现（`block_timeout`）在相同条件下返回 true（超时），确认修复有效

### 5.2 板级验证

在实际 LicheeRV Nano SG2002 工作流上重跑上传、下载和双向测试，验证中断延迟不退化。

## 6. 参考

- 评审反馈：`www/rc.md`
- `WaitQueue::wait_timeout_until` 实现：`os/arceos/modules/axtask/src/wait_queue.rs:148-179`
- `SpinNoIrq` 定义：`components/kspin/src/lib.rs:69`
- `poll_int_status` 当前实现：`components/sdhci-cv1800/src/lib.rs:160-239`
- ISR 处理：`components/sdhci-cv1800/src/irq.rs:173-212`
- ArceOS glue：`os/arceos/modules/axruntime/src/wifi_glue.rs:65-108`
