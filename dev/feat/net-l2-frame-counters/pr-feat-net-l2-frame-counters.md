# PR: feat(starry,ax-net): return L2 frame length from Device send/recv for net_stats byte counters

## 概述

将 `ax-net` 内部字节计数器从 IP payload 长度对齐到 L2 帧长度，与 Linux `/proc/net/dev` 语义一致。

## 动机

`net_stats` 暴露的字节计数器应对齐 Linux `/proc/net/dev`，即统计 L2 帧字节数（Ethernet frame，不含 FCS）。此前 `count_rx()`/`count_tx()` 传入的是 IP payload 长度（`packet.len()`），缺少 L2 头部的 14 字节（Ethernet header），对短帧还缺少 ETH_ZLEN 补齐部分。需要让 Device 层显式返回 L2 帧长度，使 router 层统计值语义正确。

## 改动

### Device trait 语义变更

`recv()` 和 `send()` 返回值从 `bool`（有无包）改为 `usize`（L2 帧字节数）：

- **`recv() -> usize`**：返回入站 IP 包的 L2 帧字节数；ARP 等非 IP 帧记录到内部延迟队列后返回 0
- **`send() -> usize`**：返回实际发出的 L2 帧字节数；排队等待（如 ARP 未就绪）或发送失败返回 0
- 新增 **`drain_deferred_tx() -> Vec<usize>`**：收集 `recv()` 期间异步发出的帧长度（ARP 请求与应答），每次调用清空内部累加器
- 新增 **`drain_deferred_rx() -> Vec<usize>`**：收集 `recv()` 期间收到的非 IP 帧长度（ARP 请求/应答及其他已通过 L2 有效性检查的帧），每次调用清空内部累加器

### 各 Device 实现

| 实现 | recv 返回值 | send 返回值 | drain_deferred |
|------|------------|------------|----------------|
| **EthernetDevice** | IP 帧：raw Ethernet frame 长度；非 IP：0（长度记入 deferred_rx_frame_lens） | 成功：补齐 ETH_ZLEN 的线缆帧长；失败：0 | TX：ARP 请求/应答帧长；RX：ARP 帧及未知 EtherType 的有效 L2 帧长 |
| **Loopback** | 原始数据长度 | 原始数据长度 | 默认空 |
| **Driver (VirtIO)** | 适配新签名 | 适配新签名 | 默认空 |

**EthernetDevice 关键细节**：
- `handle_frame()` 返回值从 `bool` 改为 `usize`（L2 帧长），非 IP 的有效 L2 帧（ARP 及未知 EtherType）记录到 `deferred_rx_frame_lens`
- `send_to()` 返回 `usize`，发送成功后记录到 `deferred_tx_frame_lens`（ARP 请求/应答通过 side channel 发送，不由 TX worker 计数）
- `set_ipv4_addr()` 在重配置前 drain 已积累的延迟帧长并原地计数，避免因 IP 地址变化丢失已成功交给设备的 TX 统计事件

### Router 层

- TX worker：`count_tx(frame_len)`，frame_len 来自 `device.send()` 返回值，仅非零时计数
- RX worker：轮询循环中 `recv()` 返回的 (packet, frame_len) 压入持久 `local_batch` FIFO，再通过 `drain_local_batch_step()` 推入共享 RX 队列；每次迭代后调用 `drain_deferred_tx()` / `drain_deferred_rx()` 统计 side channel 帧
- 新增 `drain_local_batch_step()` 辅助函数：生产 worker 与测试共用，保证 frame_len:packet 1:1 配对在背压场景下不脱钩

### 测试

**router 层 `l2_counter_tests`（12 个用例）**：

- `count_rx_accumulates_bytes_and_packets` / `count_tx_accumulates_bytes_and_packets` — 累加正确性
- `stats_starts_at_zero` / `stats_reflects_current_counters_after_counting` — 计数器生命周期
- `send_returns_frame_len_tx_counts_l2_not_ip_payload` — TX 统计 L2 帧长而非 IP payload
- `send_returns_zero_no_tx_counted` / `recv_returns_zero_no_rx_counted` — 零值不计数
- `recv_returns_frame_len_rx_counts_it` — RX 统计 L2 帧长
- `drain_deferred_tx_default_returns_empty_vec` / `drain_deferred_rx_default_returns_empty_vec` — 默认无延迟帧
- `rx_backpressure_preserves_frame_len_pairing` — 背压下 frame_len 与 packet 配对不脱钩
- `rx_worker_three_path_combined_drain` — RX worker 三条路径组合 drain

**ethernet 层 `arp_counter_tests`（9 个用例）**：

- `arp_request_tx_is_counted_in_drain_deferred_tx` / `arp_reply_tx_is_counted_in_drain_deferred_tx` — ARP TX 帧经 drain_deferred_tx 统计
- `arp_request_rx_is_counted_in_drain_deferred_rx` / `arp_reply_rx_is_counted_in_drain_deferred_rx` — ARP RX 帧经 drain_deferred_rx 统计
- `consecutive_arp_frames_accumulate_in_drain_deferred_rx` — 连续 ARP 帧累加
- `send_to_wire_len_respects_eth_zlen_padding` — ETH_ZLEN 补齐
- `combined_arp_ip_recv_drain_cycle` — ARP+IP 混合接收 drain 周期
- `set_ipv4_addr_preserves_undrained_frame_lens` — 重配置保留未 drain 的帧
- `unknown_ethertype_frame_is_counted_in_drain_deferred_rx` — 非 ARP 未知 EtherType 帧计数

## 提交

```
f5db842e8 fix(ax-net): preserve deferred counters on reconfig, count non-ARP frames, and extract drain helper
17aa03629 fix(ax-net): refine L2 frame counter drain API and add integration tests
1729ba360 style(ax-net): apply cargo fmt to router.rs l2_counter_tests
d5d50d8f0 feat(starry,ax-net): return L2 frame length from Device send/recv for net_stats byte counters
```

## 文件变更

```
 net/ax-net/src/device/driver.rs   |  13 +-
 net/ax-net/src/device/ethernet.rs | 677 +++++++++++++++++++++++++++++++++++---
 net/ax-net/src/device/loopback.rs |  13 +-
 net/ax-net/src/device/mod.rs      |  49 ++-
 net/ax-net/src/router.rs          | 563 ++++++++++++++++++++++++++++---
 5 files changed, 1227 insertions(+), 88 deletions(-)
```
