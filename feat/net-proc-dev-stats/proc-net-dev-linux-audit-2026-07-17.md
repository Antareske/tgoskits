# /proc/net/dev 实现 vs Linux 内核标准 — 严格审查

对比依据：
- [Linux networking statistics 官方文档](https://www.kernel.org/doc/html/latest/networking/statistics.html)
- [net/core/net-procfs.c](https://github.com/torvalds/linux/blob/master/net/core/net-procfs.c) (`dev_seq_printf_stats` + `dev_seq_show`)

## 一、输出格式对比

### Linux 标准（`linux/linux/net/core/net-procfs.c:49-65`）

**Header**（`dev_seq_show:75-79`）:
```
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
```
Line 1: 76 chars, Line 2: 122 chars

**每行格式**（`dev_seq_printf_stats`）:
```c
seq_printf(seq, "%6s: %7llu %7llu %4llu %4llu %4llu %5llu %10llu %9llu "
           "%8llu %7llu %4llu %4llu %4llu %5llu %7llu %10llu\n", ...);
```

**Linux 输出样例**:
```
  eth0: 1234567890123 9876543210    5    2    0     0          0         0 987654321098 8765432109    0    1    0     0       0          0
```
每行列宽对齐，解析器可靠。

### 当前实现 (`proc.rs:376-400`)

**Header**（实测输出）:
```
Inter-|   Receive                                                |                     Transmit
face |bytes    packets errs drop fifo frame compressed                    multicast|bytes    packets errs drop fifo colls carrier compressed
```
Line 1: **95** chars (Linux: 76), Line 2: **140** chars (Linux: 122)

**每行格式**:
```rust
"{:>8}: {} {} {} {} 0 0 0 0 {} {} {} {} 0 0 0 0"
```

**当前输出样例**:
```
    eth0: 1234567890123 9876543210 5 2 0 0 0 0 987654321098 8765432109 0 1 0 0 0 0
```
长度 82 char vs Linux 138 char。字段紧贴，无法按列解析。

### 格式差异汇总

| # | 项目 | Linux | 当前 | 严重度 |
|---|------|-------|------|--------|
| 1 | 接口名宽度 | `%6s` | `{:>8}` | 🟡 偏移 |
| 2 | 数值列宽 | 固定 `%7llu` `%4llu`... | 无 `{}` | 🔴 解析器不兼容 |
| 3 | Line 1 长度 | 76 | 95 (多 19 空格) | 🟡 装饰行不对齐 |
| 4 | Line 2 行首 | ` face`（有空格） | `face`（无） | 🟡 |
| 5 | Line 2 `compressed` 与 `multicast` 间距 | 1 空格 | 20 空格 | 🟡 |
| 6 | Line 2 总长 | 122 | 140 | 🟡 |

---

## 二、语义对比：rx_errors

### Linux 定义
> "Total number of **bad packets** received on this network device."  
> Must subsume `rx_length_errors`, `rx_crc_errors`, `rx_frame_errors`, and other uncategorized errors.

### 当前实现
已在以下位置计数：
- `ethernet.rs:602` — driver `receive()` 返回非 Again 错误 → `deferred_rx_errors += 1`
- `ethernet.rs:618` — `recycle_rx_buffer()` 失败 → `deferred_rx_errors += 1`

由 RX worker 汇聚：`router.rs:1096-1099`

### ❌ 缺口：handle_frame 无效帧未计为 rx_errors

`ethernet.rs:336-339`:
```rust
let Ok(repr) = EthernetRepr::parse(&frame) else {
    warn!("Dropping malformed Ethernet frame");
    return 0;  // ← 应该递增 deferred_rx_errors
};
```

对照 Linux：`rx_frame_errors` 是 "Receiver frame alignment errors"，而 `rx_errors` 是这些子错误的超集。格式错误的以太网帧属于"bad packets"，应当计入 `rx_errors`。

---

## 三、语义对比：tx_dropped（🔴 严重语义错误）

### Linux 定义（`include/uapi/linux/if_link.h`）

> `tx_dropped`: Number of packets dropped on their way to transmission, e.g. due to **lack of resources**.
>
> `tx_errors`: Total number of transmit problems. Must include `tx_aborted_errors`, `tx_carrier_errors`, `tx_fifo_errors`, `tx_heartbeat_errors`, `tx_window_errors`.

⚠️ 关键区分：`tx_dropped` 是 **资源不足无法发出**；`tx_errors` 是 **发出时硬件/驱动报错**。两者正交，不可混用。

### Device trait 定义（`device/mod.rs:86-92`）

```rust
/// Returns the L2 frame byte count (excluding FCS) actually transmitted,
/// or 0 if the packet was queued for later transmission (e.g. pending ARP
/// resolution) or could not be sent.
fn send(&mut self, next_hop: IpAddress, packet: &[u8], timestamp: Instant) -> usize;
```

**trait 注释已明确承认**：返回 0 同时表示 "pending ARP（后续会发）" 和 "could not be sent（真的失败了）"。这两个语义在统计上对应完全不同的计数器。

### `EthernetDevice::send()` 五种 return-0 路径追踪

| 路径 | 代码位置 | 语义 | 应统计 | 当前统计 |
|------|----------|------|--------|----------|
| P1 | `ethernet.rs:638-641` | broadcast `send_to()` 失败 | tx_errors | `deferred_tx_errors += 1` ✅ |
| P2 | `ethernet.rs:654-657` | unicast `send_to()` 失败（已知邻居） | tx_errors | `deferred_tx_errors += 1` ✅ |
| P3 | `ethernet.rs:668-674` | ARP 请求完全失败（无 IP 配置等） | tx_dropped | **无** ❌ |
| P4 | `ethernet.rs:675-681` | pending buffer 满 | tx_dropped | **无** ❌ |
| P5 | `ethernet.rs:682-687` | 包成功入队 pending，**后续会发** | **无** | **无** ✅ |

### ❌ TX worker 侧的连锁错误

`router.rs:1021-1025` (`device_tx_worker`):
```rust
let frame_len = device.inner.lock().send(...);
if frame_len > 0 {
    device.count_tx(frame_len);
} else {
    device.count_tx_drop();  // ← 对所有 5 种路径统一 count_tx_drop
}
```

**错误叠加**：
- **P1/P2**：`deferred_tx_errors += 1` 已计入 → worker 又 `count_tx_drop()` → **tx_dropped 虚增 + tx_errors 漏报**（在 worker 层）
- **P5**（最常见路径：ARP pending）：包已入队 pending，后续会经 `process_arp()` → `deferred_tx_frame_lens` → `drain_deferred_tx()` → `count_tx()` 正确计入 tx。但中间多了一次 `count_tx_drop()`，导致 **tx_dropped 持续虚增**
- **P3/P4**：确实应该计入 tx_dropped，但 send() 内部未设 deferred counter，全依赖 worker 的 count_tx_drop。**如果未来有其他 Device 实现不经过 TX worker 直接调用 send()，会丢失统计。**

### 根因

`Device::send()` 的 `usize` 返回值将 "pending" 和 "failed" 两类语义压缩到一个标量中，caller 无法区分。

### 正确的 tx_dropped 计数点
- `enqueue_tx()` MTU 超 (`router.rs:423`) — 资源不足 ✅
- `enqueue_tx()` TX 队列满 (`router.rs:432`) — 资源不足 ✅

### 正确的 tx_errors 计数点
- `send_to()` alloc_tx_buffer 失败 (`ethernet.rs:301`)
- `send_to()` transmit 失败 (`ethernet.rs:314`)
- `send_to()` recycle_tx_buffers 失败 (`ethernet.rs:283`)
- 以上经 `send()` 中的 `deferred_tx_errors += 1` → `drain_deferred_tx_errors()` → `count_tx_error()`

---

## 四、语义对比：rx_dropped

### Linux 定义
> "Number of packets received but **not processed**, e.g. due to lack of resources or unsupported protocol."  
> For software devices: packets dropped due to lack of host resources or unsupported protocol types.

### 当前计数点
- `router.rs:793` — `Router::poll()` 中 smoltcp RX buffer 满 → `count_rx_drop()` ✅
- `router.rs:1069` — RX worker 中接收包超过 MTU → `count_rx_drop()` ✅
- `router.rs:1103` — 共享 RX 队列满 → **重试而非计数** ⚠️

关于第 3 点：`drain_local_batch_step` 在共享队列满时返回 `Err` → worker yield + 重试。包保留在 `local_batch` 中，不会丢失。这是重试/背压策略，不是 drop。从 Linux 语义看这不算 `rx_dropped`（包并未被丢弃）。

### 潜在遗漏
- 未知 EtherType 的帧目前计入 `deferred_rx_frame_lens`（即计入 rx_packets/rx_bytes），符合 Linux "good packet received" 的语义。但如果希望更接近 Linux，可以同时计入 rx_dropped（收到但未处理）。当前行为偏"收到即算"，偏向 rx_packets。

---

## 五、tx_dropped vs tx_errors 边界混淆

当前架构中，`EthernetDevice::send()` 对两类情况都返回 0：
- ARP pending（包已入队，后续会发）— 不应该是 drop 或 error
- `send_to()` 实际失败 — 应该是 tx_error

TX worker (`device_tx_worker`) 无法区分这两种情况，统一 `count_tx_drop()`，导致：
1. **tx_dropped 虚增**：ARP pending 包被误算为 drop
2. **tx_errors 被掩盖**：实际发送失败未在 worker 层算入 tx_errors（不过在 send() 内部已计入 deferred_tx_errors，经 RX worker drain 正确计入）

根本原因是 `Device::send()` 的返回值为 `usize`，无法携带"pending vs failed"的语义。

---

## 六、架构层面的问题

### NetDevStats 结构体缺少字段
Linux rtnl_link_stats64 有 24 个字段。当前 `NetDevStats` 只有 8 个统计字段。对于软件设备，**16 个硬件字段保持 0 是正确的**，但至少应添加注释说明。

### 缺失的关键字段
`multicast` — 出现在 `/proc/net/dev` 的独立列中。当前硬编码 0，对软件设备可接受。

### render_proc_net_snmp()
已存在（`proc.rs:402-422`）且输出 Tcp/Udp 零值行，附带注释说明 smoltcp 不支持。✅ 框架正确，等 smoltcp 支持后填入。

---

## 七、问题汇总（按严重程度排列）

### 🔴 Critical — 语义错误

| # | 问题 | 位置 | 修复方向 |
|---|------|------|----------|
| 1 | TX worker 将 ARP-pending 误判为 tx_dropped | `router.rs:1025` | TX worker 不应无差别 count_tx_drop；send() 需要区分"pending"和"failed" |
| 2 | handle_frame 无效帧未计入 rx_errors | `ethernet.rs:337` | 返回值无法区分"无IP帧"和"错误帧"；需单独信号 |

### 🟡 High — 格式/兼容性

| # | 问题 | 位置 | 修复方向 |
|---|------|------|----------|
| 3 | 接口名宽度 `{:>8}` 应为 `{:>6}` | `proc.rs:387` | 对齐 Linux `%6s` |
| 4 | 数值字段无固定宽度 | `proc.rs:387` | 改为 Linux 对应的定宽格式 |
| 5 | Header 行首缺空格（`face` vs ` face`） | `proc.rs:377` | 对齐 Linux header |

### 🟢 Low — 注释/文档

| # | 问题 | 位置 |
|---|------|------|
| 6 | 硬件字段为何为 0 缺少注释 | `proc.rs:387` |
| 7 | multicast 硬编码 0 无说明 | `proc.rs:387` |

---

## 八、`Device::send()` 返回值语义问题根因

当前 trait 定义 (`device/mod.rs`):
```rust
fn send(&mut self, next_hop: IpAddress, packet: &[u8], timestamp: Instant) -> usize;
```

返回 `usize` 表示 L2 帧长度，0 表示"未发送"。但 0 承载了两种不同语义：
- **pending**：包已入队（ARP 等待），后续会发
- **failed**：发送失败（资源不足、硬件错误等）

这两种情况在统计上对应完全不同的计数器：
- pending → 不产生任何统计事件（包尚未离开）
- failed → tx_errors（发送路径出错）或 tx_dropped（资源不足）

**建议方向**：要么改成 `Result<usize, SendError>`，要么由 `send()` 内部（而非 caller）负责统计决策。

---

## 九、修复方案

### 修复 1 — 输出格式对齐 Linux（`proc.rs`）

Header 和每行格式严格对齐 Linux `dev_seq_show` / `dev_seq_printf_stats`：

Header:
```rust
"Inter-|   Receive                                                |  Transmit\n \
  face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs \
 drop fifo colls carrier compressed\n"
```

每行（注意 Linux 用的宽度规格，对变量用 Rust 的 `{:>width$}` 语法）：
```rust
write!(buf, "{:>6}: {:>7} {:>7} {:>4} {:>4} {:>4} {:>5} {:>10} {:>9} \
             {:>8} {:>7} {:>4} {:>4} {:>4} {:>5} {:>7} {:>10}\n",
    st.name,
    st.rx_bytes, st.rx_packets, st.rx_errors, st.rx_dropped,
    0u64 /*fifo*/, 0u64 /*frame*/, 0u64 /*compressed*/, 0u64 /*multicast*/,
    st.tx_bytes, st.tx_packets, st.tx_errors, st.tx_dropped,
    0u64 /*fifo*/, 0u64 /*colls*/, 0u64 /*carrier*/, 0u64 /*compressed*/,
);
```

### 修复 2 — malformed 帧计入 rx_errors（`ethernet.rs`）

`handle_frame()` 中对无效帧递增 `deferred_rx_errors`：

```rust
let Ok(repr) = EthernetRepr::parse(&frame) else {
    warn!("Dropping malformed Ethernet frame");
    self.deferred_rx_errors += 1;
    return 0;
};
```

### 修复 3 — tx_dropped 虚增（`router.rs` + `ethernet.rs`）

**核心思路**：由 `EthernetDevice::send()` 内部对所有失败路径做完整统计，TX worker 不再对 return-0 做任何假设。

**步骤**：

1. `EthernetDevice` 新增 `deferred_tx_drops: u64` 字段，在 `send()` 的 P3/P4 路径递增：
   - P3 (ARP 请求失败): `self.deferred_tx_drops += 1`
   - P4 (pending buffer 满): `self.deferred_tx_drops += 1`
   - P4 (enqueue 失败): `self.deferred_tx_drops += 1`

2. `Device` trait 新增 `drain_deferred_tx_drops()` 方法（与 `drain_deferred_tx_errors()` 对称）。

3. RX worker 中添加 drain：
   ```rust
   let tx_drops = device_inner.drain_deferred_tx_drops();
   for _ in 0..tx_drops {
       device.count_tx_drop();
   }
   ```

4. **TX worker 改为**：只对 send() > 0 做 count_tx，不对 return-0 做任何统计：
   ```rust
   let frame_len = device.inner.lock().send(packet.next_hop, packet.bytes.as_slice(), now());
   if frame_len > 0 {
       device.count_tx(frame_len);
   }
   // return-0 → 不做统计，send() 内部已完成所有 counting
   ```

### 修复 4 — 硬件字段注释（`proc.rs`）

在 `render_proc_net_dev()` 格式串中添加注释说明 fifo/frame/colls/carrier/compressed/multicast 为 0 的原因。
