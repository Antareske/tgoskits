# PR: 使用 XFER_COMPLETE 中断修复 SG2002 WiFi 上行吞吐量

## 背景

SG2002 (LicheeRV Nano) 的 aic8800 WiFi 模组通过 SDIO 接口与 SoC 通信，SDHCI 驱动使用 PIO 模式，PIO 传输完成等待（`poll_int_status`）采用两阶段轮询：

- **Phase 1**：纯寄存器自旋 1000 次（约 50µs）
- **Phase 2**：最多 20 万次循环，每圈 `yield_now()` 让出 CPU 后重新检查 INT_STATUS

上行方向存在严重吞吐问题，见 [StarryOS WiFi 上行吞吐排查报告](https://docs.qq.com/doc/DTkN3Y1JOeWhJVGdV)。报告发现，硬件在 t≈250µs 就发完帧并置好了 XFER_COMPLETE。本 PR 认为可以通过使能 XFER_COMPLETE 的中断来控制 CPU 返回驱动以修复上行吞吐量极低的问题，无需让出时间片或忙等 XFER_COMPLETE 置位。

## 方案与改动

核心思路：用 XFER_COMPLETE 硬件中断替代 `yield_now()`——任务阻塞在 WaitQueue，硬件完成传输后 ISR 立即唤醒，唤醒延迟从约 48ms 降至微秒级。

### 1. 修复 WiFi 启动

aic8800 RX 线程的周期性 kicker 原先仅在 dual-pipe 模式下启动。改为无条件启动 10ms kicker，作为 ISR 驱动 RX 响应路径不可靠时的兜底，确保入站帧不丢失。此改动使得单通道的 aic8800D80 WiFi 芯片可在 starryos 启动时顺利初始化 AP 模式。

> **提交**：`fix&test(sg2002,wifi): fix wifi boot & include sg2002 wifi config`
>*该提交包含一处对 sg2002 配置文件的改动，已追加提交回退。

### 2. XFER_COMPLETE 中断

在 Phase 2 中使能 XFER_COMPLETE 中断信号，任务通过 `block_timeout` 阻塞在 WaitQueue，硬件完成时 SDHCI ISR 写 `NORM_INT_SIG_EN` 清零 XFER_COMPLETE 使能位并调用 `notify_one_from_irq` 唤醒任务，任务醒来后在 `poll_status_once` 中读到 XFER_COMPLETE 已置位，W1C 清除后返回。

XFER_COMPLETE 是 sticky bit，若 ISR 提前 W1C 清除，任务醒来看到状态为 0 会误判未完成，所以 ISR 只 mask 信号不碰状态位，清除统一由 `poll_status_once`（`poll_int_status` 内）执行。

此前 `clear_stale_status` 在每条命令前 W1C 清理 INT_STATUS 残留位，在实现了 XFER_COMPLETE 中断后会误清 XFER_COMPLETE ，故清理残留位时过滤该位。此外，非 XFER 等待的错误/超时退出路径也需要处理未受理的 XFER_COMPLETE，否则会被下一次 `wait_transfer_complete` 当成提前成功，因此改为退出路径加清。

> **提交**：
> - `fix(sdhci-cv1800): add interrupt-driven PIO transfer completion`
> - `fix(sdhci-cv1800): use selective W1C to preserve XFER_COMPLETE across command and error paths`
> - `fix(sdhci-cv1800): consume XFER_COMPLETE in error and timeout exit paths`

### 3. Store Buffer Fence

PIO 写 `SDHCI_BUFFER` 后，写操作可能会被 CPU store buffer 暂存而非直接输出到总线（SDHCI 寄存器是 MMIO，CPU 有可能会乱序执行不同内存地址的操作）。若 `pio_write` 操作被暂存，进入 Phase 1 后 CPU 自旋读 `INT_STATUS`，妨碍滞后的 `pio_write` 及时输出到总线（竞争总线），硬件收到 `pio_write` 并置位 `INT_STATUS` 的时间被推迟，导致 Phase 1 的 50µs 窗口可能在硬件就绪前过期。实测表现如此，上行吞吐极小（略好于引入 XFER_COMPLETE 中断之前）。

在 Phase 1 入口加 `fence(SeqCst)` 排空 store buffer 再轮询。原本的 `yield_now()` 没这个问题，其任务切换的 `mret` 自带 fence 语义，换成阻塞等待后需要显式补上。

> **提交**：`fix(sdhci-cv1800): drain store buffer before Phase 1 MMIO polling`

## 测试结果

**纯上传（TX）**

```
[ ID] Interval           Transfer     Bitrate         Retr  Cwnd
[  5]   0.00-1.00   sec  1.12 MBytes  9.43 Mbits/sec    0   0.00 Bytes
[  5]   1.00-2.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   2.00-3.00   sec  1.12 MBytes  9.44 Mbits/sec    0   0.00 Bytes
[  5]   3.00-4.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   4.00-5.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   5.00-6.00   sec  1.00 MBytes  8.39 Mbits/sec    0   0.00 Bytes
[  5]   6.00-7.00   sec  1.12 MBytes  9.43 Mbits/sec    0   0.00 Bytes
[  5]   7.00-8.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   8.00-9.00   sec  1.25 MBytes  10.5 Mbits/sec    0   0.00 Bytes
[  5]   9.00-10.00  sec  1.12 MBytes  9.44 Mbits/sec    0   0.00 Bytes
- - - - - - - - - - - - - - - - - - - - - - - - -
[  5]   0.00-10.02  sec  11.9 MBytes  9.94 Mbits/sec    0            sender
```

**纯下载（RX）**

```
[ ID] Interval           Transfer     Bitrate
[  5]   0.00-1.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   1.00-2.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   2.00-3.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   3.00-4.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   4.00-5.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   5.00-6.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   6.00-7.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   7.00-8.00   sec  1.25 MBytes  10.5 Mbits/sec
[  5]   8.00-9.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   9.00-9.94   sec  1.25 MBytes  11.2 Mbits/sec
- - - - - - - - - - - - - - - - - - - - - - - - -
[  5]   0.00-9.94   sec  13.0 MBytes  11.0 Mbits/sec                  receiver
```

**双向**

```
[ ID][Role] Interval           Transfer     Bitrate         Retr  Cwnd
[  5][RX-S]   0.00-1.00   sec   512 KBytes  4.19 Mbits/sec
[  8][TX-S]   0.00-1.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   1.00-2.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   1.00-2.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   2.00-3.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   2.00-3.00   sec   768 KBytes  6.29 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   3.00-4.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   3.00-4.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   4.00-5.00   sec   768 KBytes  6.29 Mbits/sec
[  8][TX-S]   4.00-5.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   5.00-6.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   5.00-6.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   6.00-7.00   sec   640 KBytes  5.25 Mbits/sec
[  8][TX-S]   6.00-7.00   sec   768 KBytes  6.30 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   7.00-8.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   7.00-8.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   8.00-9.00   sec   640 KBytes  5.24 Mbits/sec
[  8][TX-S]   8.00-9.00   sec   640 KBytes  5.24 Mbits/sec    0   0.00 Bytes
[  5][RX-S]   9.00-9.96   sec   640 KBytes  5.49 Mbits/sec
[  8][TX-S]   9.00-9.96   sec   640 KBytes  5.49 Mbits/sec    0   0.00 Bytes
- - - - - - - - - - - - - - - - - - - - - - - - -
[  5][RX-S]   0.00-9.96   sec  6.25 MBytes  5.27 Mbits/sec                  receiver
[  8][TX-S]   0.00-9.96   sec  6.50 MBytes  5.48 Mbits/sec    0            sender
```

## 改动内容

### 提交列表

| 提交 | 说明 |
|------|------|
| `fix&test(sg2002,wifi): fix wifi boot & include sg2002 wifi config` | RX kicker 无条件启动 |
| `fix(sdhci-cv1800): add interrupt-driven PIO transfer completion` | XFER_COMPLETE 中断驱动核心实现 |
| `fix(sdhci-cv1800): drain store buffer before Phase 1 MMIO polling` | Phase 1 入口 atomic fence |
| `fix(sdhci-cv1800): use selective W1C to preserve XFER_COMPLETE` | 选择性 W1C，保护 XFER_COMPLETE |
| `fix(sdhci-cv1800): consume XFER_COMPLETE in error and timeout exit paths` | 错误/超时路径消费 XFER_COMPLETE |
| `chore(sdhci-cv1800): translate newly-introduced comments to Chinese` | 注释中文化 |
| `chore(sg2002): remove aic8800-wifi feature from licheerv-nano-sg2002 base config` | 回退 base config |

### 改动文件

| 文件 | 说明 |
|------|------|
| `components/aic8800/src/fdrv/thread/rx.rs` | RX kicker 无条件启动 |
| `components/sdhci-cv1800/src/irq.rs` | XFER_COMPLETE ISR、CallbackSlot、RMW 集中化 |
| `components/sdhci-cv1800/src/lib.rs` | poll_int_status 两阶段重构、fence、选择性 W1C |
| `components/sdhci-cv1800/src/regs.rs` | SIG_MASK 仅使能 CARD_INT |
| `components/sdhci-cv1800/src/runtime.rs` | SdhciDelay trait：yield_now → block_timeout |
| `os/arceos/modules/axruntime/src/wifi_glue.rs` | block_timeout 实现、PIO wake callback |
| `os/StarryOS/configs/board/licheerv-nano-sg2002.its` | WiFi 启动修复（ITS 设备树） |
