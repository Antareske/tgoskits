# PR: feat/net-proc-dev-stats — /proc/net/dev error/drop 计数器 & /proc/net/snmp 骨架

## 问题

StarryOS 的 `/proc/net/dev` 仅输出 `rx_bytes`/`rx_packets`/`tx_bytes`/`tx_packets`，其余 12 列（errors、dropped、fifo、frame、compressed、multicast、colls、carrier）全部为 0 且无实际计数来源。同时缺少 `/proc/net/snmp`，导致依赖这些伪文件系统的用户空间网络工具（如 `netstat -i`、`ifconfig`、`ss`）无法正常工作或显示不完整。

## 做了什么

以 Linux `rtnl_link_stats64` 语义和 `/proc/net/dev` 格式为参照，实现了完整的 error/drop 计数器数据通路。

### 三层数据通路

```
EthernetDevice::send() ──失败──▶ deferred_tx_errors/tx_drops ──drain──▶ DeviceHandle::count_tx_error/drop()
EthernetDevice::recv() ──错误──▶ deferred_rx_errors/rx_drops ──drain──▶ DeviceHandle::count_rx_error/drop()
DeviceHandle enqueue/drop ──直接──▶ count_rx_drop()/count_tx_drop()
```

### 各字段计数来源

| 字段 | 计数场景 |
|:---|:---|
| `rx_errors` | malformed Ethernet/ARP 帧、驱动 receive 错误、recycle_rx_buffer 失败 |
| `tx_errors` | send_to 硬件失败（alloc/transmit）、ARP 协议失败（IPv6 不支持、IP 未配置） |
| `rx_dropped` | 未知 EtherType 帧、loopback 注入失败、RX buffer 满、over-MTU RX |
| `tx_dropped` | pending buffer 满/入队失败、TX queue 满、over-MTU TX、路由查找失败 |

### /proc/net/dev 格式

采用与 Linux `dev_seq_printf_stats()` 完全一致的 17 列宽度。硬件专用字段（fifo、frame、compressed、multicast、colls、carrier）显式置零并注释——QEMU virtio 无硬件事件源。

### /proc/net/snmp

提供带正确格式头部的骨架文件，TCP/UDP 计数器返回 0（smoltcp 0.13.1 不暴露逐协议累计计数器）。真实值待后续网络栈升级后填充。

## 关键设计决策

1. **Loopback 先注入后计数**：将 `count_tx`/`count_rx` 移至 `inject_loopback_rx*` 成功之后，避免 RX buffer 满时计数器虚高。
2. **ARP 失败归入 tx_errors**：ARP 请求无法发出本质是设备层传输失败，区别于资源约束导致的 tx_drops。
3. **未知 EtherType 同时计入 rx_packets 和 rx_dropped**：匹配 Linux 对同一帧两个计数器均递增的行为。
4. **TX worker 不再盲目计丢包**：`send() == 0` 可能是 ARP pending（非丢包），错误/丢弃统一由设备内部累计、RX worker 排空，消除 double-counting 风险。
5. **`set_ipv4_addr` 保留未排空累加器**：符合 Linux "counters survive routine interface operations" 语义。

## 测试

`cargo test -p ax-net` — 68 个测试全部通过。新增 4 个针对性测试覆盖：
- malformed Ethernet 帧 → `rx_errors`
- malformed ARP payload → `rx_errors`
- pending buffer 满 → `tx_drops`
- send_to alloc 失败 → `tx_errors`

`cargo clippy` 通过（base + vsock features）。
