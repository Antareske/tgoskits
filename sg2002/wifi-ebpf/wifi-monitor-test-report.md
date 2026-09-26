# wifi_monitor eBPF 探针测试报告

**平台**：SG2002 LicheeRV Nano + AIC8800DC WiFi (SDIO)
**镜像**：`licheerv-nano-sg2002-wifi.toml` (log=Error, RX kicker 修复, kprobe 优雅报错)
**测试日期**：2026-08-01

## 测试方法

```bash
wifi_monitor <采样秒数> &     # 后台启动探针
sleep 2
iperf3 -s -1                  # PC 端 iperf3 -c <板子IP>
# 等待 wifi_monitor 采样结束并输出
```

## 探针覆盖

| 探针 | 类型 | 目标 | 监测内容 |
|------|------|------|------|
| tx_enqueue | kprobe | `enqueue_data_frame` | TX 入队帧数 + 以太网字节 |
| sdio_write_entry | kprobe | `SdioTransport::write_fifo` | SDIO 写入入口（记录时间戳 + 字节长度） |
| sdio_write_return | kretprobe | `SdioTransport::write_fifo` | SDIO 写入返回延迟 + 错误检测 |

## 延迟直方图分桶

| 桶 | 范围 | 诊断意义 |
|---|---|---|
| 0 | <300µs | 认为是正常硬件传输 |
| 1 | 300–500µs | 轻度延迟 |
| 2 | 500µs–1ms | |
| 3 | 1–5ms | |
| 4 | 5–20ms | 周期性延迟 (如 SLOWWRITE) |
| 5 | 20–50ms | **sched-rr 50ms 时间片放大** |
| 6 | 50–100ms | 1–2 个时间片 |
| 7 | ≥100ms | 极端延迟 |

## 测试结果

### 测试 1：板子下载流量

| 指标 | 值 |
|------|-----|
| iperf3 TCP 下行 | **9.77 Mbps** (11.8 MB / 10s) |
| TX_ENQUEUE cnt | 3617 帧 |
| TX_ENQUEUE bytes | 217,627 (平均 60 B/帧，ACK 模式) |
| SDIO_WR bytes | 1,851,904 |
| SDIO_WR err/kretprobe 触发 | 7234 (平均每 TX 帧 2 次 SDIO 写) |

**延迟直方图**：

```
SDIO_WR_LAT  3617 0 0 0 0 0 0 0 (total=3617)
```

下行时所有流量都正常，没有出现延迟。（没有考虑压测情况）

### 测试 2：板子上传流量，撞进调度时间片

同样的 WiFi 连接、同样的 iperf3 命令，但吞吐量骤降：

| 指标 | 值 |
|------|-----|
| iperf3 TCP 上行 | **82.5 Kbps** (128 KB / 12.7s) |
| TX_ENQUEUE cnt | 331 帧 |
| TX_ENQUEUE bytes | 476,644 (平均 1440 B/帧) |
| SDIO_WR bytes | 492,032 |
| SDIO_WR err/kretprobe 触发 | 662 |

**延迟直方图**：

```
SDIO_WR_LAT  17 0 0 0 0 314 0 0 (total=331)
```

| 桶 | 计数 | 占比 |
|---|---|---|
| 0 (<300µs) | 17 | 5.2% |
| **5 (20–50ms)** | **314** | **94.8%** |

94.8% 的 SDIO 写入被 20-50ms 的时间片拖慢了，这与邵志航老师的发现一致。目前辰龙机器人使用 tgoskits 主线，仍有这个问题。

## 结论

已完成的 eBPF 成功捕获 SDIO 写入延迟分布。另外也观察到有无探针挂载时的性能差异：探针激活时 iperf3 仍达 9.77 Mbps，与无探针时 (~12 Mbps) 相比下降约 19%，主要是每次 SDIO 写触发 kprobe + kretprobe + rbpf 去解释执行 eBPF 程序所导致。
