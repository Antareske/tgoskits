# SG2002 WiFi 上下行 iperf3 表现分析

## 实验环境

- **硬件**: LicheeRV Nano (SG2002, C906 单核 RISC-V)
- **WiFi 模组**: aic8800 (SDIO 4-bit, 25MHz)
- **内核**: StarryOS (基于当前 `sg2002/wifi-irq` 分支)
- **TX 中断**: 已实现（XFER_COMPLETE 走硬件中断唤醒，CMD_COMPLETE / BUF_WR_READY 仍为轮询）
- **SDIO 模式**: PIO（非 DMA）
- **网络拓扑**: Starry 作为 AP (192.168.50.1)，测试客户端 (192.168.50.2) 通过 WiFi 连接
- **iperf3**: Starry 侧运行 `iperf3 -s`（服务端），客户端通过 `-R` 控制上下行方向

## 测试方法

- **TX 测试（上行）**: Starry 作为 iperf3 sender，向客户端发送数据
- **RX 测试（下行）**: Starry 作为 iperf3 receiver，接收客户端数据

两次测试分别在 Starry 上独立运行 `iperf3 -s`，由客户端控制方向。

## 原始数据

### TX (上行, Starry → Client)

```
Server listening on 5201 (test #1)
Accepted connection from 192.168.50.2, port 59998
[  5] local 192.168.50.1 port 5201 connected to 192.168.50.2 port 60002
[ ID] Interval           Transfer     Bitrate         Retr  Cwnd
[  5]   0.00-1.01   sec   128 KBytes  1.03 Mbits/sec    0   0.00 Bytes
[  5]   1.01-2.00   sec   128 KBytes  1.06 Mbits/sec    0   0.00 Bytes
[  5]   2.00-3.03   sec   256 KBytes  2.03 Mbits/sec    0   0.00 Bytes
[  5]   3.03-4.01   sec   128 KBytes  1.07 Mbits/sec    0   0.00 Bytes
[  5]   4.01-5.03   sec   128 KBytes  1.03 Mbits/sec    0   0.00 Bytes
[  5]   5.03-6.00   sec   128 KBytes  1.07 Mbits/sec    0   0.00 Bytes
[  5]   6.00-7.01   sec  0.00 Bytes  0.00 bits/sec    0   0.00 Bytes
[  5]   7.01-8.02   sec   128 KBytes  1.04 Mbits/sec    0   0.00 Bytes
[  5]   8.02-9.03   sec  0.00 Bytes  0.00 bits/sec    0   0.00 Bytes
[  5]   9.03-10.01  sec   128 KBytes  1.07 Mbits/sec    0   0.00 Bytes
[  5]  10.01-11.02  sec  0.00 Bytes  0.00 bits/sec    0   0.00 Bytes
[  5]  11.02-11.12  sec  0.00 Bytes  0.00 bits/sec    0   0.00 Bytes
- - - - - - - - - - - - - - - - - - - - - - - - -
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-11.12  sec  1.12 MBytes   848 Kbits/sec    0            sender
```

关键特征：`Retr` 和 `Cwnd` 列存在，starry 为 sender，iperf3 输出 TCP 发送端统计。

### RX (下行, Client → Starry)

```
Server listening on 5201 (test #1)
Accepted connection from 192.168.50.2, port 60000
[  5] local 192.168.50.1 port 5201 connected to 192.168.50.2 port 60012
[ ID] Interval           Transfer     Bitrate
[  5]   0.00-1.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   1.00-2.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   2.00-3.00   sec  1.38 MBytes  11.5 Mbits/sec
[  5]   3.00-4.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   4.00-5.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   5.00-6.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   6.00-7.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   7.00-8.00   sec  1.50 MBytes  12.6 Mbits/sec
[  5]   8.00-9.00   sec  1.12 MBytes  9.44 Mbits/sec
[  5]   9.00-10.00  sec  1.50 MBytes  12.6 Mbits/sec
- - - - - - - - - - - - - - - - - - - - - - - - -
[ ID] Interval           Transfer     Bitrate
[  5]   0.00-10.01  sec  14.4 MBytes  12.0 Mbits/sec                  receiver
```

关键特征：无 `Retr`/`Cwnd` 列，starry 为 receiver，iperf3 输出 TCP 接收端统计。

## 现象分析

### 数据对比

| 指标 | TX (上行) | RX (下行) |
|------|-----------|-----------|
| 总吞吐量 | 848 Kbps | 12.0 Mbps |
| 传输总量 | 1.12 MB / 11.12s | 14.4 MB / 10.01s |
| 每秒波动 | 0 ~ 256 KB，剧烈抖动 | 1.12 ~ 1.50 MB，稳定 |
| 重传 (Retr) | 0 | N/A |
| Cwnd 终值 | 0.00 Bytes | N/A |
| TX/RX 不对称比 | — | **~14 倍** |

### TX Burst-Gap 模式

TX 测试呈现规律的间歇性突发：

- 每段 burst 多为 **128 KB**（偶尔 256 KB），在约 1 秒内发出
- 随后 1–2 秒完全静默（0 字节）
- 整个 10 秒窗口约一半间隔为 0

### Retr=0 与 Cwnd=0 的组合含义

- **Retr=0**: 无线链路质量不差，无丢包触发 TCP 重传。问题不在传输层丢包。
- **Cwnd=0.00 (终值)**: 测试终点 TCP 拥塞窗口为 0，发送端处于阻塞状态。这不表示整个测试期间 CWND 始终为 0（否则不会有数据发出），但说明**数据流在测试结束时刻被卡住**，无法继续发送。

两者共同指向：瓶颈性质是**流控/缓冲阻塞**，而非无线传输丢包。

## 根因分析

### TX 路径流程

```
write_fifo() → cmd53_write_fixed() → cmd53_xfer()
  ├─ wait_data_idle()              // 自旋等 CMD+DAT 空闲
  ├─ wait_cmd_complete()           // Phase1 自旋 + Phase2 10ms 睡眠轮询 (非中断驱动)
  └─ pio_write()                   // 逐 block 写 SDHCI_BUFFER
       └─ wait_buffer_write_ready() // Phase1 自旋 + Phase2 10ms 睡眠轮询 (非中断驱动)
  └─ wait_transfer_complete()      // Phase1 自旋 + Phase2 中断驱动 ✓ (本次实现)
```

### TX 中断改进已覆盖的部分

`wait_transfer_complete()` 的 XFER_COMPLETE 在 Phase 2 走硬件中断唤醒（`block_timeout` → `WaitQueue::wait_timeout`），消除了此前每次传输末尾 10ms×20=200ms 的睡眠轮询开销。此改进已生效。

### 尚未被中断覆盖的等待点

`poll_int_status` 中仅 XFER_COMPLETE 走中断路径：

- **CMD_COMPLETE**: Phase 2 仍为 10ms 睡眠轮询（最多 200ms 超时）。CMD53 启动后等待命令完成确认。
- **BUF_WR_READY**: Phase 2 仍为 10ms 睡眠轮询。每 512B block 需等待一次。

对一次 128 KB TX（256 blocks），`pio_write` 调用 256 次 `wait_buffer_write_ready()`。虽然绝大部分在 Phase 1（1000 次自旋 ≈ 50µs）内命中，但硬件偶尔慢于 Phase 1 窗口时即掉入 10ms 睡眠——累积延迟可观。

### 128 KB Burst 成因

128 KB 与 aic8800 firmware 的 TX buffer 容量高度吻合：

1. **上层快速灌入**: TCP/IP 栈持续将数据推入 WiFi 驱动 → firmware TX buffer
2. **Buffer 满**: firmware TX buffer 达到上限（~128 KB），流控触发，上层被阻塞
3. **Buffer 排空**: Firmware 通过 SDIO PIO 将 buffer 内容发出（约需 1 秒 @ ~1 Mbps 有效速率）
4. **上层继续**: Buffer 排空后流控释放，上层灌入下一批 128 KB → 回到步骤 1

RX 方向不出现此问题：数据流向为"拉"模式，firmware 有数据时通过 CARD_INT 通知驱动主动 PIO read，消费者（iperf3 → socket）持续消费，RX buffer 不会积压到触发流控。

### PIO 模式的限制

SG2002 使用 PIO（非 DMA）模式进行 SDIO 传输。每个 32-bit 字需要一个 MMIO store/load 指令：

- TX 128 KB = 32K 次 `write_volatile` store
- RX 1.5 MB/s = 384K 次 `read_volatile` load/s

在 C906 单核（~1 GHz）上，MMIO 操作与 TCP 协议栈、WiFi 驱动、中断处理竞争 CPU 时间。

**RX 能达到 12 Mbps 而 TX 只有 0.85 Mbps 的原因**：
- RX 是"拉"模式：数据主动流入，链路通畅时不会反压
- TX 是"推"模式：firmware buffer 满时，整个管道反压到应用层

RX 的 12 Mbps 证明了 PIO 本身可以达到此速率——前提是数据流不被流控打断。

### TCP CWND 与流控的交互

TX 路径间歇性导致：
1. 一批数据发出 → TCP 等待对端 ACK
2. 等待期间 TX 路径被流控阻塞 → 无法发送新数据段，也无法及时发送对端数据的 ACK
3. 有效 RTT 剧烈波动 → TCP 难以准确估算可用带宽 → CWND 增长受限
4. 测试终点 CWND=0：测试结束时 TX 正处于一次流控阻塞中

## 与忙等方案的对照

### Part 1 忙等方案的测试结果

来自 `sg2002-wifi-performance-analysis.md`（基于 BattiestStone4 的测试记录）：

| 里程碑 | TX 上行 | 方案 |
|--------|---------|------|
| Part 1 起点 | 0.2 Mbps | legacy-g + PIO 默认等待 |
| Part 1 终点 | **~10 Mbps** | SDHCI busy-wait（`yield_now()` 自旋） |
| 当前分支 | **0.85 Mbps** | XFER_COMPLETE 中断驱动 + 其余位 `delay_ms(10)` 睡眠 |

### 为什么忙等反而更快？

忙等方案的核心是所有等待点（CMD_COMPLETE / BUF_WR_READY / XFER_COMPLETE）都用 `yield_now()` 自旋：

```
查状态 → 没好 → yield_now() → 查状态 → ...
          ↑ 空闲系统上 yield 立即返回，微秒级间隔
```

当前方案的非 XFER 位用的是 `delay_ms(10)`：

```
查状态 → 没好 → sleep(10ms) → 查状态 → ...
          ↑ 硬件就绪后最多白等 10ms
```

对于单核 TX 场景，CPU 没有其他任务可做——它唯一的目标就是把数据尽快推出去。**自旋严格优于睡眠**：

- 一次 128KB TX = 256 blocks
- 即使 90% 在 Phase 1（50µs）命中，10%（~25 blocks）落入 Phase 2
- 忙等：25 × ~50µs ≈ **1.25ms** 额外等待
- 当前：25 × 10ms ≈ **250ms** 额外等待 — **200 倍差距**

**0.85 Mbps vs 10 Mbps 的差距（~12×），`delay_ms(10)` 的粗粒度睡眠是直接原因之一。**

注意：Part 1 的 10 Mbps 还包含 50MHz SDIO、HT 对齐修复、流控优化等其他改进，不在当前分支。但即使仅比较等待粒度，10ms 睡眠 vs 微秒级自旋的差距也是数量级的。

## 结论

| 维度 | 结论 |
|------|------|
| TX 吞吐 | 848 Kbps，约为 RX 的 1/14，约为 Part 1 忙等方案的 1/12 |
| 丢包 | 无，无线链路质量不差 |
| 瓶颈性质 | **双重因素**：(1) `BUF_WR_READY`/`CMD_COMPLETE` 的 10ms 粗粒度睡眠拖慢了数据注入节奏；(2) firmware TX buffer 流控反压放大了等待的影响 |
| TX 中断改进效果 | XFER_COMPLETE 中断驱动已生效，消除了最坏 200ms 超时等待 |
| 剩余关键路径 | `BUF_WR_READY` / `CMD_COMPLETE` 的 Phase 2 需恢复快速轮询（`yield_now` 自旋或缩短至 1ms 以下），而非当前 10ms 睡眠 |
| RX 表现 | 12 Mbps 稳定，PIO 读 + CARD_INT 中断通路正常 |
| 核心判断 | XFER_COMPLETE 中断改进是正确的（消除了 200ms 尾部等待），但 **非 XFER 位的 10ms 睡眠替换是 TX 退化的直接原因**——对于单核 TX 热路径，自旋优于睡眠 |
