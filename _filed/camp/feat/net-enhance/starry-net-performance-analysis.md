# Starry 网络栈性能与稳定性全面分析

> 目标：系统梳理 StarryOS（`net/ax-net` + `drivers/net` + smoltcp）的网络实现，定位制约
> 单核 / 多核吞吐与稳定性的瓶颈，给出追赶 Linux 的可执行工作项。
>
> 本分析基于当前 dev 分支状态（`ax-net`，smoltcp 0.13.1）。
> 所有结论均标注了对应源码位置，便于复核。
>
> **更新说明**：标注"✅ 已完成"的项目已在 dev 分支中完成；其余为待推进项。

---

## 1. 架构现状速览

```
应用 / syscall
   │  send/recv/accept/connect/poll
   ▼
ax-net socket 层   tcp.rs / udp.rs / raw.rs / unix / vsock
   │  通过 request_poll() 请求协议推进
   ▼
专属 net-poll worker ← ✅ 已完成
   │  驱动 Interface.poll
   ▼
全局服务 SERVICE: Once<Mutex<Service>>
全局套接字 SOCKET_SET: Mutex<SocketSet>     ← 仍为单把大锁
   │
   ▼
smoltcp Interface.poll(timestamp, &mut Router, &mut SocketSet)
   │
   ▼
Router (phy::Device)   单一 rx_buffer / tx_buffer（各 64 包）
   │  多接口支持、per-interface 路由 ← ✅ 已完成
   ▼
EthernetDevice   ARP/neighbor、pending_packets，SpinNoIrq<Box<dyn EthernetDriver>>
   │
   ▼
RdNetDriver → rd-net (Net/TxQueue/RxQueue)  单队列、DMA pool
   │
   ▼
virtio-net / e1000 / ixgbe / realtek / fxmac
```

关键全局对象（`net/ax-net/src/lib.rs:116-130`）：

- `SERVICE: Once<Mutex<Service>>`：整个协议栈一把锁。
- `SOCKET_SET: Mutex<SocketSet>`：所有 socket 一把锁。
- `LISTEN_TABLE: LazyLock<ListenTable>`：HashMap 索引监听端口 ← ✅ 已优化。
- 专属 net-poll worker：通过 `NET_POLL_REQUESTED` / `NET_POLL_WAKE` 解耦数据面 ← ✅ 已完成。
- `DEFERRED_POLL_WAKES`：延迟 PollSet 通知，避免在持锁时触发 waker ← ✅ 已完成。

**核心判断：当前实现已从"同步内联 poll"演进为"异步 poll worker + deferred waker"模型。** 
poll 模型重构解决了 syscall 路径与协议推进的解耦问题，但数据面仍存在全局大锁、4+ 次拷贝、
单队列等系统性瓶颈。

---

## 2. 单核性能瓶颈

### 2.1 收包路径存在 4 次以上内存拷贝

一个入站 TCP 数据包从网卡到用户态，经历的拷贝（RX 方向）：

1. **DMA → 驱动私有 Vec**：`RdNetDriver::receive` 里 `packet.to_vec()`
   （`drivers/net/rd-net/src/lib.rs:443-447`、`net/ax-net/src/device/driver.rs:158-161`）。
2. **驱动 Vec → Router rx_buffer**：`EthernetDevice::handle_frame` 中
   `buffer.enqueue(...).copy_from_slice(frame.payload())`（`device/ethernet.rs:277-282`）。
3. **rx_buffer → smoltcp socket 接收环**：`Interface.poll` 经 `RxToken` 拷入 socket buffer。
4. **socket 环 → 用户缓冲**：`TcpSocket::recv` 的 `socket.recv(|buf| dst.write(buf))`
   （`tcp.rs:536-542`）。

TX 方向类似：用户 → socket 环 → `tx_buffer`（`TxToken`，`router.rs:260-269`）→
`EthernetDevice::send` 的 `pending_packets` 或 `alloc_tx_buffer` 拷贝 → 驱动 staging。
virtio 还会再加一层 staging 拷贝（`drivers/ax-driver/src/virtio/net.rs:196-204`，每包
`alloc::vec![0; header+len]` + `copy_from_slice`）。

对比 Linux：典型 RX 是 DMA 进 `skb` 后仅一次 `copy_to_user`；TX 走
`sendpage`/`MSG_ZEROCOPY` 可做到零拷贝。**Starry 每包多出 2–3 次拷贝，是单核吞吐最直接的损耗。**

**可做的工作：**
- 用引用 / 所有权传递替换 `to_vec()`：让 `NetRxBuffer` 直接承载 DMA buffer，`Device::recv`
  把 buffer 的所有权（或借用切片）交给上层，去掉第 1 次拷贝。
- 让 `Router` 的 `RxToken` 直接引用驱动 buffer，绕过中转 `rx_buffer`，合并第 2、3 次拷贝。
- virtio TX 去掉 staging Vec：预留 header 空间，直接在 DMA buffer 内组帧。
- 长期：评估 smoltcp 的 `socket-tcp` 是否可改用外部 ring，减少 socket 环这一层拷贝。

### 2.2 RX 没有批处理，每包一次加锁

`RX_PREFETCH_TARGET = 1`（`device/driver.rs:6`），`receive()` 每次只取 **一个** 包，且每个包
都要 `self.inner.driver.lock()`（`device/ethernet.rs:482-507`）加解锁一次。`Router::poll`
是 `while ... dev.recv(...)` 单包循环（`router.rs:150-153`）。

这意味着：没有 NAPI 式的“一次中断收多包”批量收割，锁与函数调用开销按包线性放大。

**可做的工作：**
- 把 `RX_PREFETCH_TARGET` 提到 32/64，`receive` 改为返回一批 buffer，`Router::poll` 批量入队。
- 驱动层一次 `lock()` 收割整轮 ring（参考 NAPI budget），减少每包加锁。

### 2.3 poll 模型（✅ 已大幅改善，但仍有优化空间）

**✅ 已完成：**
- 引入专属 net-poll worker，socket 方法通过 `request_poll()` + waker 异步推进协议栈。
- `DEFERRED_POLL_WAKES` 机制避免在持锁时触发 waker，防止 PollSet 唤醒死锁。
- 数据面与 syscall 路径解耦，不再每个 `send`/`recv` 都内联驱动整栈。
- IRQ-safe 延迟通知支持，保证中断上下文安全。

**仍需优化：**
- 缺少 NAPI/softirq 风格的 budget 轮询与中断合并，中断频率仍较高。
- 批量收包能力尚未实现（见 2.2）。

**可做的工作：**
- 引入 budget poll（每次 poll 最多处理 N 个包）+ 中断合并（interrupt coalescing）。

### 2.4 软件校验和，无任何 offload

`smoltcp` feature 未启用任何硬件 offload；raw 路径甚至显式
`ChecksumCapabilities::ignored()`（`raw.rs:315`），而正常路径用默认软件校验和。整个
IP/TCP/UDP 校验和都在 CPU 上算。没有 TSO/GSO/GRO、没有 jumbo frame（MTU 固定 1500，
`consts.rs:1`、`router.rs:341`）。

对比 Linux：校验和 offload、TSO/GRO 是万兆吞吐的基础。

**可做的工作：**
- 为支持 offload 的网卡（virtio-net 的 `VIRTIO_NET_F_CSUM/GSO`、e1000/ixgbe）打通
  `DeviceCapabilities.checksum`，让 smoltcp 跳过对应软件校验和。
- 评估 GRO（接收侧合并）以减少协议栈每包开销——这是单核小包场景收益最大的一项。

### 2.5 每包堆分配

`VecTxBuffer::new` / `VecRxBuffer`（`device/driver.rs:86-120`）每包 `alloc::vec!`；
virtio staging 每包分配；ARP pending 重排时 `buf.to_vec()`（`ethernet.rs:424-427`）。
高频路径上的分配器压力会放大 P99 延迟。

**可做的工作：**引入每队列的 buffer 池 / slab，复用收发缓冲（rd-net 已有
`ContiguousBufferPool`，可上提到中间层复用）。

---

## 3. 多核扩展性瓶颈

### 3.1 全局大锁仍是首要问题（部分已改善）

**✅ 已改善：**
- poll 模型重构后，数据面不再由每个 syscall 内联驱动，减少了锁持有时间。
- `LISTEN_TABLE` 改为 HashMap 索引，端口查找不再线性扫描 65536 个槽位。

**仍待解决：**
- `SERVICE: Mutex<Service>`：任何收发、ARP、路由、DHCP 都要拿这把锁。
- `SOCKET_SET: Mutex<SocketSet>`（`lib.rs:117`）：任何 socket 的读写状态都要拿这把锁。
- 锁序固定为 `SERVICE -> SOCKET_SET -> listen entry`（`listen_table.rs` 注释已明确）。

在 N 核同时收发时，协议栈仍大部分串行，**多核几乎不带来吞吐提升**，反而因锁竞争 + cache
line 争用而下降。

**对比 Linux：** 每 CPU 的 softirq、RPS/RFS、per-socket 锁、SO_REUSEPORT 多队列监听、
per-CPU 内存池，使吞吐近似随核数线性扩展。

**可做的工作（按收益排序）：**
1. **拆分 SOCKET_SET 锁**：从"一把全局锁"改为 per-socket 锁 / 分片锁（sharded by handle）。
   socket 收发只锁自己，互不阻塞。这是多核扩展的关键一步。
2. **多队列 + RSS**：驱动开多 RX/TX 队列（现在 `QUEUE_ID0` 单队列），按流哈希分配到不同核，
   每核独立 poll 自己的队列。
3. **per-CPU / per-queue 的 Router/Interface 实例**或至少把设备 RX 的临界区下沉到队列级，
   避免 `SERVICE` 这把全局锁覆盖数据面。
4. **SO_REUSEPORT** 风格的多 listener 分流，配合多队列分担 accept。

### 3.2 端口分配的全局争用（部分已改善）

**✅ 已改善：**
- `LISTEN_TABLE` 改为 HashMap，按端口哈希索引，不再遍历 65536 个数组槽位。

**仍需优化：**
- `get_ephemeral_port` 单把 `Mutex<u16>` + 线性扫描（`tcp.rs` 类似位置）。
- `udp_bind_check` 遍历全部 socket（`wrapper.rs` 类似位置，已标 `TODO: optimize`）。
- `TCP_BOUND_PORTS` 全局 `Mutex<HashMap>` 线性查重。

**可做的工作：** 端口分配改用 per-CPU 区间 / 位图；bind 查重改用哈希索引而非线性扫描。

### 3.3 waker 机制（✅ 已优化）

**✅ 已完成：**
- `DEFERRED_POLL_WAKES` 机制避免在持 SOCKET_SET 锁时直接触发 PollSet waker。
- waker 延迟到 net-poll worker 外层循环统一处理，避免重入和死锁。
- 统一 deferred waker 设计，替代了旧的 `Service::register_waker` 频繁分配。

**旧问题（已解决）：**
- `Service::register_waker` 每次都 `Box::pin(sleep_until(...))` 重建定时器 future，
  高频 poll 下产生分配与 drop。多核下叠加锁竞争更明显。

---

## 4. 稳定性风险

数据面里的 `panic!` / `expect` / `unwrap` 在内核态会直接拖垮系统，属于高优先级稳定性隐患：

| 位置 | 风险 | 触发条件 |
|------|------|----------|
| `service.rs:451` `get_source_address` `panic!("no route...")` | 内核 panic | connect 到无路由地址 |
| `router.rs:224,245` `assert_eq!(rule.src, ...)` | 内核 panic | 源地址与路由不一致（伪造/竞态） |
| `router.rs:209,211,232` `IpVersion::of_packet().expect()` | 内核 panic | 畸形 IP 包入 tx_buffer |
| `tcp.rs:255` `LISTEN_TABLE.can_accept(...).unwrap()` | 内核 panic | listener 状态竞态 |
| `router.rs:268` `TxToken` `.expect("checked before")` | 内核 panic | 缓冲竞态 |

**丢包/限容路径**（功能正确但影响稳定吞吐）：

- `SOCKET_BUFFER_SIZE = 64`（`consts.rs:11`）：Router 收发环各仅 64 包，突发即满即丢。
- `Router::poll` 中 `!self.rx_buffer.is_full()` 满了就停收（`router.rs:150-152`），中断侧无背压
  反馈，易丢包。
- ARP pending 满即丢（`ethernet.rs:549-552`），注释已说明 32 太小提到 128，但仍是定值。
- `SendBuffer`/`ReceiveBuffer` 的 setsockopt 是 **no-op**（`general.rs:197-199`、
  `tcp.rs:297-302` 固定返回 64KB），应用无法调优窗口；缺少接收窗口自动调整（auto-tuning）。

**其他：**

- `connect` 里 `ax_task::yield_now()`（`tcp.rs:404` “Hack: let the server listen”）——靠让出
  CPU 等待，时序脆弱。
- `dns_query` 是 `poll + yield_now` 忙等循环（`lib.rs:527-550`）。
- DHCP bootstrap 固定 200×10ms 轮询（`lib.rs:560-568`）。

**可做的工作：**
- 把数据面所有 `panic/expect/unwrap` 改为丢包 + 计数 + 告警，绝不让畸形包/竞态打死内核。
- 加入 per-device / per-socket 统计计数（rx/tx/drop/retrans/queue-full），既利于调优也利于定位。
- 实现 `SO_SNDBUF/SO_RCVBUF` 与接收窗口自动调整，提升大带宽时延积链路吞吐。
- 给 Router 收发环加可配置容量与背压（满时通知驱动停 NAPI，而非静默丢）。

---

## 5. 与 Linux 的能力差距汇总

| 能力 | Linux | Starry 现状 | 差距 |
|------|-------|-------------|------|
| 拷贝次数 (RX) | ~1 (copy_to_user) | 4+ | 高 |
| 多核扩展 | per-CPU softirq + RPS/RFS | 全局大锁 + 单飞 poll | 高 |
| 多队列 / RSS | 是 | 单队列 | 高 |
| 中断模型 | NAPI 批量 + 合并 | 中断仅 wake，内联 poll | 高 |
| 校验和 offload | 是 | 软件计算 | 中 |
| TSO/GSO/GRO | 是 | 无 | 中 |
| 零拷贝 (sendfile/MSG_ZEROCOPY) | 是 | 无 | 中 |
| 缓冲自动调整 | 是 | 固定 64KB，setsockopt no-op | 中 |
| jumbo frame | 是 | MTU 固定 1500 | 低 |
| 数据面健壮性 | 丢包不崩 | 多处 panic 路径 | 高（稳定性） |

---

## 6. 建议的工作路线（按性价比）

### 阶段一：稳定性与低风险加速（改动局部、收益确定）—— 约 40% 完成

**✅ 已完成：**
- poll 模型重构：专属 net-poll worker、数据面与 syscall 解耦、deferred waker
- listen_table 优化：HashMap 索引替代线性扫描
- 多接口支持：per-interface 路由、环回快速路径
- 并发文档：878 行锁顺序与竞态说明

**待推进：**
1. 清除数据面所有 `panic/expect/unwrap`，改为丢包 + 计数。（第 4 节表格）
2. RX 批处理：`RX_PREFETCH_TARGET` 调大 + 批量收割，减少每包加锁。（2.2）
3. 去掉 `connect` 的 yield hack 等时序脆弱路径。（4）
4. 增加 per-socket / per-device 统计计数，建立可观测性基线。（4）
5. 提升并使 Router 收发环容量可配置，加背压。（4）

### 阶段二：单核吞吐（减少拷贝与 CPU 开销）—— 约 10% 完成

6. 收包路径去掉 `to_vec()`，buffer 所有权直传，合并中转拷贝。（2.1）
7. virtio TX 去 staging 拷贝；引入每队列 buffer 池复用。（2.1、2.5）
8. 打通校验和 offload；评估 GRO。（2.4）
9. 实现 `SO_SNDBUF/SO_RCVBUF` 与接收窗口自动调整。（4）

### 阶段三：多核扩展（架构级，收益最大但改动最深）—— 约 20% 完成

**✅ 已完成：**
- poll 线程解耦为多核扩展打好基础
- listen_table HashMap 优化

**待推进：**
10. 拆分 `SOCKET_SET` 全局锁为 per-socket / 分片锁。（3.1）
11. 驱动多队列 + RSS，按流分核；每核独立 poll。（3.1）
12. 引入 budget poll + 中断合并。（2.3、3.1）
13. 端口分配 / bind 查重去线性扫描，per-CPU 化。（3.2）
14. SO_REUSEPORT 多 listener 分流。（3.1）

> 说明：阶段三的多核改造（尤其 #10、#11）是"追赶 Linux 吞吐"的决定性工作；前两阶段则是
> 风险可控、能较快兑现的改进，且为阶段三打好可观测性与正确性基础。**当前综合完成度约 30%。**

---

## 7. 验证与度量建议

- **基线压测**：单核 / 多核分别用 iperf3（TCP/UDP 单流与多流）、nginx + wrk（短连接 QPS）、
  小包 PPS（pktgen 风格）跑出当前数值，后续每项改动都对比。
- **微基准**：统计单包 RX/TX 的拷贝次数与 CPU cycles（可用计数器或 trace）。
- **多核扩展曲线**：固定负载、逐步增核，观察吞吐是否随核数提升——这是验证 3.1 改造成效的核心指标。
- **稳定性回归**：畸形包 / 满队列 / 无路由 connect 等异常输入下系统不崩（对应第 4 节）。
- 复用现有 `test-suit/starryos` 网络用例与 `smoltcp-fuzz` 做正确性与健壮性回归。
