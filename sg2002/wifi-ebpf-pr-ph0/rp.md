# SG2002 WiFi 上行 eBPF 监测

**网卡**：AIC8800D80

commit: 
[7b7c275](https://github.com/Antareske/tgoskits/commit/7b7c275ba42e777347a51e90529e41f347c78bb4) |
[56e2b41](https://github.com/Antareske/tgoskits/commit/56e2b4119c9d7d58d731336ffe0489e7d0f8df81) |
[d101690](https://github.com/Antareske/tgoskits/commit/d101690e9945200ba3f520d968174b931a2949de)

## 说明

之前尝试使用 raw tracepoint 的方式监控驱动的方式不合理，对源码改动过大。现在统一改用 kprobe/kretprobe 探针，主要监测 WiFi 上行传输被 CPU 调度卡住的行为。

## 功能

### TX 帧计数 TX_ENQUEUE (kprobe)

探针类型：kprobe。

目标函数：`enqueue_data_frame`，位于 `components/aic8800/src/fdrv/thread/tx.rs:548`。该函数是 TX 路径的入口，负责将以太网帧封装为 802.11 帧并提交到 SDIO 发送队列。

统计每次 WiFi 帧入队的帧数和以太网字节数。

### SDIO 写入字节与错误计数 SDIO_WR (kprobe, kretprobe)

探针类型：kprobe（入口）+ kretprobe（返回）。

目标函数：`SdioTransport::write_fifo`，位于 `components/aic8800/src/fdrv/core/sdio_transport.rs:166`。该函数是 SDIO 传输层的核心写入接口，TX 数据帧、WPA2 握手帧、启动配置等均通过此函数经 CMD53 下发到 SDIO 设备。

统计每次 SDIO write_fifo 提交的字节数，以及写入失败的次数。

### SDIO 写延迟直方图 SDIO_WR_LAT (kretprobe)

目标函数：`SdioTransport::write_fifo`，同上。

记录每次 write_fifo 从进入到返回的耗时，按 8 个尺度统计延迟（μs）：<300, <500, <1k, <5k, <20k, <50k, <100k, ≥100k。

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

## 探针覆盖

| 探针 | 类型 | 目标 | 监测内容 |
|------|------|------|------|
| tx_enqueue | kprobe | `enqueue_data_frame` | TX 入队帧数 + 以太网字节 |
| sdio_write_entry | kprobe | `SdioTransport::write_fifo` | SDIO 写入入口（记录时间戳 + 字节长度） |
| sdio_write_return | kretprobe | `SdioTransport::write_fifo` | SDIO 写入返回延迟 + 错误检测 |

## 测试方法

```bash
wifi_monitor <采样秒数> &     # 后台启动探针
sleep 2
iperf3 -s -1                  # PC 端 iperf3 -c <板子IP>
# 等待 wifi_monitor 采样结束并输出
```

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

**注意这是 tx**。虽然监控的是 tx，但可见下行时 tx 流量都正常，没有出现延迟。（没有考虑压测情况）

### 测试 2：板子上传流量，撞进调度时间片

及其卡顿，连命令行都显示地极慢：

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
