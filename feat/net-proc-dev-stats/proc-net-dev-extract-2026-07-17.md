# /proc/net/dev 完善 — 从 todo-plan 提取（2026-07-17）

提取自 `feat/net-enhance/www/todo-plan-2026-07-17.md`，仅保留 StarryOS 内核侧 `/proc/net/dev` 完善的本体任务。

## 当前状态

- `os/StarryOS/kernel/src/pseudofs/proc.rs:376-393` — `render_proc_net_dev()` 渲染标准 Linux 格式，含 `bytes` 和 `packets`。
- `NetDevStats` 仅有 `rx_bytes/rx_packets/tx_bytes/tx_packets` 4 字段（`net/ax-net/src/router.rs:86-93`）
- `DeviceHandle` 仅有 4 个 `AtomicU64` 计数器（`router.rs:281-284`）
- **问题**：`errors`、`drops`、`fifo` 等列全部硬编码为 `0`，无对应计数基础设施。

## 任务：补充 /proc/net/dev 缺失维度

### drops 计数

- **RX drops**：在 `device_rx_worker()` 中，当 `local_batch` 满、`buffer.enqueue()` 失败时计数
- **TX drops**：在 `device_tx_worker()` 中，当 `Device::send()` 返回 0（非 pending ARP 情况）时计数
- 在 `DeviceHandle` 中新增 `rx_drops: AtomicU64`、`tx_drops: AtomicU64`

### errors 计数

- **RX errors**：在 `EthernetDevice::handle_frame()` 中，无效 EtherType、校验失败等路径计数
- **TX errors**：在 `EthernetDevice::send_to()` 中，发送失败路径计数

### NetDevStats 扩展

- 新增 `rx_errors/rx_drops/tx_errors/tx_drops` 字段
- 更新 `render_proc_net_dev()` 输出，替换硬编码的 0

### 硬件相关字段（保持 0）

`fifo`、`frame`、`compressed`、`multicast`、`colls`、`carrier` 是硬件相关字段，QEMU virtio 环境暂无对应事件源，保持 0 并添加注释说明。

## 涉及文件

- `net/ax-net/src/router.rs` — `NetDevStats`、`DeviceHandle`、worker 计数点
- `net/ax-net/src/device/ethernet.rs` — RX errors 计数
- `os/StarryOS/kernel/src/pseudofs/proc.rs` — `render_proc_net_dev()`
