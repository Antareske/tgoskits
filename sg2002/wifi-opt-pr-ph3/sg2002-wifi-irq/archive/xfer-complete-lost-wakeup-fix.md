# SDHCI XFER_COMPLETE 丢唤醒窗口的修复与效果

## 背景

SG2002 的 SDHCI 以 PIO 模式与 AIC8800 WiFi 芯片通信，CMD53 传输完成后由 XFER_COMPLETE 中断唤醒阻塞任务。该等待路径存在一个丢唤醒窗口：任务重开 XFER_COMPLETE 中断信号（unmask）后、进入 WaitQueue 前，传输可能已经完成，ISR 的通知因任务尚未入队而落空，任务只能睡满 10ms 超时后才发现完成。上传路径每 128KB 需数百次 CMD53 传输、每次都等待一次 XFER_COMPLETE，窗口命中时吞吐与尾延迟受损。本次改动把条件检查与任务入队收拢到同一关中断临界区，关闭该窗口；同时补上错误状态位的 STS_EN 使能与确定性回归测试。

## 整体流程

```
任务侧 poll_int_status() Phase 2            [lib.rs]
  │
  ├─ unmask_xfer_complete_signal()          [irq.rs]  （SIG_EN 置位 XFER_COMPLETE）
  ▼
block_timeout_until(10ms, 条件)             [runtime.rs]
  ▼
ArceosDelay::block_timeout_until            [wifi_glue.rs]
  └─ SDHCI_PIO_WQ.wait_timeout_until()
       ├─ 关中断（WQ 自旋锁）
       ├─ 检查条件: INT_STATUS_NORM & (bit | ERROR)   [lib.rs]
       └─ 条件未满足 → 持锁入队 → 睡眠
ISR 三种时序（单核）:
  ① 锁内检查前触发 → notify 落空，sticky 位被锁内条件观察到
  ② 检查后入队前触发 → 不可能（临界区关中断）
  ③ 入队后触发 → notify 命中队列中的任务
任务被唤醒或超时后 recheck → W1C 消费 sticky 位  [lib.rs]
```

## 具体实现

### 1. SdhciDelay::block_timeout_until — 带条件的阻塞等待

文件: `components/sdhci-cv1800/src/runtime.rs`

```rust
fn block_timeout_until(&self, timeout_ms: u64, condition: &dyn Fn() -> bool) -> bool {
    if condition() {
        return false;
    }
    self.delay_ms(timeout_ms);
    true
}
```

- 替换原来的 block_timeout（无条件睡满超时）：阻塞当前任务直至 condition 满足或超时。
- 契约要求：条件的最终检查与任务入队必须发生在同一关中断临界区内，使"检查后、入队前 ISR 已发布完成事件"不可能出现——ISR 在检查前触发由 condition 观察到其 latch 的状态位，在入队后触发则 notify 命中已入队的任务。
- 调用方在中断开启的 task 上下文调用，负责先重新打开中断信号（unmask）；关中断临界区由实现自行建立。
- 默认实现退化为"检查一次 + 睡满超时"，兼容未更新为条件等待的 OS 胶水层。

### 2. poll_int_status Phase 2 — 条件等待 + 重检消费

文件: `components/sdhci-cv1800/src/lib.rs`

```rust
irq::unmask_xfer_complete_signal();
let timed_out =
    crate::runtime::delay().block_timeout_until(PHASE2_STEP_MS, &|| {
        let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        norm & (bit | NORM_INT_ERROR) != 0
    });
if timed_out {
    log::trace!("[SDHCI] poll_int Phase-2 IRQ wait timed out: bit=0x{:04x}", bit);
}
```

- 条件同时纳入被等待位与错误位：锁内检查前已锁存的错误可即时返回；错误不产生中断（SIG_EN 不含 error 位），睡期中段到达的错误仍由 10ms 超时后的重检查出。
- ISR 在 unmask 后、锁内检查前触发时 notify 落空，但它不消费的 XFER_COMPLETE sticky 位被锁内条件立即观察到，任务无需等满超时。
- 返回后沿用原有的 post-wake recheck，W1C 消费状态位。

### 3. ArceosDelay::block_timeout_until — WaitQueue 条件等待

文件: `os/arceos/modules/axruntime/src/wifi_glue.rs`

```rust
fn block_timeout_until(&self, timeout_ms: u64, condition: &dyn Fn() -> bool) -> bool {
    SDHCI_PIO_WQ.wait_timeout_until(Duration::from_millis(timeout_ms), condition)
}
```

- wait_timeout_until 在 WQ 自旋锁（关中断）内先检查条件、未满足时持锁将任务入队后才切换，条件检查与入队在同一临界区内衔接。
- 同一时刻至多一个任务（TX 或 RX）阻塞于此——SDIO 总线锁（SdioTransport）序列化所有传输。

### 4. 错误状态位 STS_EN 使能

文件: `components/sdhci-cv1800/src/regs.rs`

```rust
pub const NORM_INT_ENABLE_MASK: u16 = NORM_INT_CMD_COMPLETE
    | NORM_INT_XFER_COMPLETE
    | NORM_INT_BUF_WR_READY
    | NORM_INT_BUF_RD_READY
    | NORM_INT_CARD_INT
    | NORM_INT_ERROR;
```

- 按 SDHCI 规范，INT_STATUS 位仅在对应 STS_EN 置位时锁存——此前掩码缺 NORM_INT_ERROR（bit15），错误检测路径（poll_status_once 错误分支与 Phase 2 条件错误项）恒假。
- 错误位不进入 SIG_EN，不产生中断，仅由轮询/条件检查消费；中断线行为不受影响。

### 5. 确定性回归测试

文件: `components/sdhci-cv1800/src/lib.rs`

- FakeIrqDelay 在 block_timeout_until 入口重放"unmask 之后、锁内检查之前"窗口内的事件，覆盖两个场景：
  - 场景 A（ISR 先于入队触发）: 硬件置位 XFER_COMPLETE，ISR mask 信号并 notify（队列为空，通知落空）——断言返回 Ok 且零睡眠，锁定"事件由锁内条件对 sticky 位的观察补偿"。
  - 场景 B（错误位锁存）: 硬件锁存 NORM_ERROR + DAT_TIMEOUT——断言立即返回错误且零睡眠，并配套 STS_EN 读回断言（掩码必须含 bit15，否则场景无意义）。
- replay 入口断言 SIG_EN 已含 XFER 位，拦截"阻塞先于 unmask"的顺序回归。
- 变异实验验证敏感度：删除 unmask 调用、条件恒假、从 STS_EN 掩码移除错误位，测试均必然失败。

## 测试结果

平台：LicheeRV Nano SG2002，网卡：aic8800D80。客户端（192.168.50.2）经 AP 连接板端（192.168.50.1），iperf3 服务端运行于板端。

### 纯下载（客户端 → 板端）

```
[ ID] Interval           Transfer     Bitrate
[  5]   0.00-1.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   1.00-2.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   2.00-3.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   3.00-4.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   4.00-5.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   5.00-6.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   6.00-7.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   7.00-8.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   8.00-9.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   9.00-9.58   sec   896 KBytes  12.6 Mbits/sec
- - - - - - - - - - - - - - - - - - - - - - - -
[ ID] Interval           Transfer     Bitrate
[  5]   0.00-9.58   sec  12.8 MBytes  11.2 Mbits/sec                  receiver
```

### 纯上传（板端 → 客户端）

```
[ ID] Interval           Transfer     Bitrate         Retr  Cwnd
[  5]   0.00-1.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   1.00-2.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   2.00-3.00   sec  1.38 MBytes  11.5 Mbits/sec    0   0.00 Bytes
[  5]   3.00-4.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   4.00-5.00   sec  1.38 MBytes  11.5 Mbits/sec    0   0.00 Bytes
[  5]   5.00-6.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   6.00-7.00   sec  1.38 MBytes  11.5 Mbits/sec    0   0.00 Bytes
[  5]   7.00-8.00   sec  1.38 MBytes  11.5 Mbits/sec    0   0.00 Bytes
[  5]   8.00-9.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   9.00-9.55   sec   768 KBytes  11.5 Mbits/sec    0   0.00 Bytes
- - - - - - - - - - - - - - - - - - - - - - - -
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-9.55   sec  12.5 MBytes  11.0 Mbits/sec    0            sender
```

### 双向

```
[ ID][Role] Interval           Transfer     Bitrate         Retr  Cwnd
[  5][RX-S]   0.00-1.00   sec   512 KBytes  4.19 Mbits/sec
[  8][TX-S]   0.00-1.00   sec   768 KBytes  6.29 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   1.00-2.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   1.00-2.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   2.00-3.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   2.00-3.00   sec   768 KBytes  6.29 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   3.00-4.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   3.00-4.00   sec   768 KBytes  6.29 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   4.00-5.00   sec   768 KBytes  6.28 Mbits/sec
[  8][TX-S]   4.00-5.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   5.00-6.00   sec   640 KBytes  5.25 Mbits/sec
[  8][TX-S]   5.00-6.00   sec   768 KBytes  6.30 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   6.00-7.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   6.00-7.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   7.00-8.00   sec   768 KBytes  6.29 Mbits/sec
[  8][TX-S]   7.00-8.00   sec   768 KBytes  6.29 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   8.00-9.00   sec   640 KBytes  5.25 Mbits/sec
[  8][TX-S]   8.00-9.00   sec   640 KBytes  5.25 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   9.00-9.56   sec   384 KBytes  5.60 Mbits/sec
[  8][TX-S]   9.00-9.56   sec   384 KBytes  5.60 Mbits/sec    0   0.00 Bytes
- - - - - - - - - - - - - - - - - - - - - - - -
[ ID][Role] Interval           Transfer     Bitrate         Retr
[  5][RX-S]   0.00-9.56   sec  6.12 MBytes  5.37 Mbits/sec                  receiver
[  8][TX-S]   0.00-9.56   sec  6.62 MBytes  5.81 Mbits/sec    0            sender
```

- 上传逐秒吞吐稳定在 10.5-11.5 Mbits/sec，全程无停顿间隔——与修复前上传方向"发一秒、停一秒"的 burst-gap 模式形成对比，丢唤醒窗口关闭后每次传输完成均由锁内条件检查或 notify 即时观察到。
- 双向两方向逐秒吞吐稳定，带宽共享均衡，无方向饿死；各方向均无重传。
- 全部测试期间 0 条 [SDHCI] poll_int mid-timeout（warn）：Phase 2 未出现累积超时的严重退化。
- 全部测试期间 0 条 [SDHCI] CMD/DAT CRC error / timeout（error）：STS_EN 使能后激活的错误检测路径在正常流量下无误报。

## 改动文件清单

| 文件 | 说明 |
|------|------|
| `components/sdhci-cv1800/src/runtime.rs` | SdhciDelay trait：block_timeout → block_timeout_until（条件等待契约 + 默认退化实现） |
| `components/sdhci-cv1800/src/lib.rs` | Phase 2 条件等待改造、timed_out trace 观测、确定性回归测试 |
| `components/sdhci-cv1800/src/irq.rs` | unmask 协议与三情形分析文档同步 |
| `components/sdhci-cv1800/src/regs.rs` | NORM_INT_ENABLE_MASK 加入 NORM_INT_ERROR（STS_EN 门控修复） |
| `os/arceos/modules/axruntime/src/wifi_glue.rs` | block_timeout_until 接线到 WaitQueue::wait_timeout_until |
