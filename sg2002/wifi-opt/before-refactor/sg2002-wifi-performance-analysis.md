# StarryOS WiFi 性能差距根因分析

> 基于 `/workspace/sg2002-wifi.md`（BattiestStone4 / 邵志航 著）与全栈静态代码分析。
> 分析日期：2026-07-17
> 基准分支：`test/verify-net-wakeup-fix`（Part 2 终点）

## 1. 背景与基线

StarryOS 在 SG2002（cv1812cp）+ AIC8800DC WiFi over SDIO 上，TCP 上行单路达到 **13.7 Mbps**，同条件 vendor Linux 为 **33.2 Mbps**（HE-MCS9, 114.7 Mbps PHY）。差距 **2.4×**，StarryOS 仅达 Linux 的 41%。

全程提升回顾（来自作者文档）：

| 里程碑 | 上行单路 | 提升倍数 | vs Linux |
|---|---|---|---|
| Part 1 起点（legacy-g） | 0.2M | 1× | 0.6% |
| Part 1 终点（SDHCI busy-wait） | ~10M | 50× | ~30% |
| + HT 对齐 + 1ms kicker | 12.7M | 64× | ~38% |
| + 50MHz SDIO + PHY + 修空转 | 13.7M | 69× | 41% |

> **注意**：Part 1（SDHCI busy-wait 修复）和 Part 2（HT 对齐、50MHz SDIO、流控修复、kicker 优化）的代码提交位于 `test/verify-net-wakeup-fix` 分支，尚未合入主线 dev 分支。本文的分析起点为 Part 2 终点状态。

---

## 2. 端到端数据路径（完整 7 层）

### 2.1 TX 路径

```
Userspace (iperf3 send)
  └→ socket write → smoltcp TCP socket buffer (64KB)
      └→ net-poll worker: Service::poll() → iface.poll()
          └→ Router::dispatch() → route lookup → DeviceHandle::enqueue_tx()
              └→ Copy 1: IP packet → QueuedPacket ([u8; 1500] 内联)
                  └→ Device TX worker: tx_queue.pop() → device.send()
                      └→ EthernetDevice::send() → ARP 查找 → send_to()
                          └→ driver.alloc_tx_buffer() → Vec<u8> 堆分配
                          └→ Copy 2: QueuedPacket → Ethernet frame Vec<u8>
                              └→ RdNetDriver::transmit() → tx_queue.prepare_send()
                                  └→ Copy 3: Vec<u8> → DMA buffer (write_with_cpu)
                                      └→ AicTxQueue::submit() (device.rs:232-244)
                                          └→ Copy 4: DMA buffer → Vec<u8> (.to_vec())
                                          └→ enqueue_data_frame() → bus.tx.queue (256 槽)
                                              └→ wifi-tx poll task: tx_process()
                                                  └→ process_data_tx() [CMD 优先, 然后 DATA batch]
                                                      └→ send_single_data_frame()
                                                          └→ build_data_frame()
                                                              └→ Vec<u8> 堆分配 (~1536B)
                                                              └→ Copy 5: eth_frame → SDIO frame buffer
                                                          └→ check_data_flow_control() → CMD52 (5-10µs)
                                                          └→ transport.write_fifo() → sdio.lock()
                                                              └→ CviSdhci::write_fifo()
                                                                  └→ cmd53_write_fixed()
                                                                      └→ cmd53_xfer()
                                                                          └→ wait_data_idle()
                                                                          └→ write BLOCK_SIZE/COUNT/ARG/CMD
                                                                          └→ wait_cmd_complete()
                                                                      └→ pio_write()
                                                                          └→ per block: wait BUF_WR_READY
                                                                          └→ 128 × write::<u32>(SDHCI_BUFFER)
                                                                      └→ wait_transfer_complete() (~212µs)
                                                                          └→ poll_int_status(XFER_COMPLETE)
                                                                              └→ Phase 1: 3ms spin
                                                                              └→ Phase 2: yield fallback
```

### 2.2 RX 路径

```
SDIO CARD_INT (IRQ#38 on SG2002)
  └→ sdio1_irq_handler → sdhci_irq_handler → set irq_pending
      └→ wifi-rx poll task 被唤醒
          └→ read_fifo_data()
              └→ Copy 1: SDIO FIFO → Vec<u8> (堆分配)
          └→ parse 802.11 frame → build_and_enqueue_eth_frame()
              └→ Copy 2: 802.11 MPDU → Ethernet frame Vec<u8> (堆分配)
          └→ push to bus.rx.data_queue → invoke_rx_data_callback()
              └→ wake_net_task_irq() → NET_IRQ_NOTIFY → wake net-poll worker
                  └→ device_rx_worker: drain device RX queue
                      └→ Copy 3: data_queue frame → DMA buffer
                  └→ RdNetDriver::receive() → prefetch_rx_packets()
                      └→ Copy 4: DMA buffer → VecRxBuffer (packet.to_vec())
                  └→ EthernetDevice::recv()
                      └→ Copy 5: Ethernet payload → DevicePacketBuffer
                  └→ Router RX enqueue
                      └→ Copy 6: DevicePacketBuffer → QueuedPacket ([u8; 1500])
          └→ Router::poll() → smoltcp
              └→ Copy 7: QueuedPacket → smoltcp rx_buffer
```

**总计**：TX 路径 **5 次全帧拷贝 + 3 次堆分配**；RX 路径 **7 次全帧拷贝 + 多次堆分配**。

### 2.3 关键测量数字

| 指标 | 值 | 来源 |
|---|---|---|
| `wait_transfer_complete()` 单笔耗时 | ~212µs（3ms busy-wait 正常捕获） | sg2002-wifi.md Part 1 |
| 固件排空速率 | ~1500 帧/秒（avg_ampdu 8-10×） | sg2002-wifi.md §12 |
| TX 线程 CPU busy | 12-14%（50MHz SDIO） | sg2002-wifi.md §12 |
| MPDU 间隔 | ~6.3ms（理论上可 ~3ms） | sg2002-wifi.md §12 |
| 空口利用率 | 40-60% | sg2002-wifi.md §12 |
| SDIO 总线频率 | 25MHz（dev）/ 50MHz（Part 2 分支） | regs.rs |
| 每帧 PIO MMIO 操作 | 384 次（3 blocks × 128 words） | sdhci-cv1800 lib.rs |
| TX 队列容量 | 256 槽 | consts.rs |
| TX 批处理上限 | 64 帧/轮询周期 | consts.rs |
| 流控阈值 | 2 credits | consts.rs |
| CPU 调度器 | sched-rr, 50ms 时间片 | StarryOS kernel Cargo.toml |

---

## 3. 根因拆解

### 3.1 第一层：PHY 速率 — HE 完全未启（~60% 差距，主因）

**代码位置**：`components/aic8800/src/fdrv/protocol/config.rs:102-129`

```rust
// ME_CONFIG_REQ 结构:
// [0..26]   mac_htcapability  (26 bytes) → HT 已配置，AMPDU 参数已设置
// [26..38]  mac_vhtcapability (12 bytes) → 全零
// [38..92]  mac_hecapability  (54 bytes) → 全零  ← HE 完全禁用
// [92..]    tail: tx_lft, phy_bw, ht_supp, vht_supp, ...

// phy_bw 对 AIC8800DC 设为 PHY_CHNL_BW_20（仅 20MHz）
// ht_supp = true, vht_supp = false
// he_cap: 54 字节全零
```

**PHY 速率对比**：
- StarryOS：HT-MCS7 @ 20MHz, 1SS → **65 Mbps**
- Linux：HE-MCS9 @ 20MHz, 1SS → **114.7 Mbps**
- 差距：**1.77×**，这是单因子最大差距

**Part 2 实验记录**（sg2002-wifi.md §10.5）：
- 曾启用 `he_supp = 1`，固件在 AssocReq 中生成了 HE 扩展 IE（`FF:23`）
- 但 HE 数据通路断（ampdu=0, DHCP 不通）
- 三次上板验证（25MHz/50MHz/修空转）均未解决
- 最终 `he_supp = 1` 被注释回退

**判断**：这不是"HE 没协商"，而是"协商后固件内部 HE TX path 不工作"。属于固件兼容性问题。可能原因：
- HE 消息结构体自然对齐问题（类似 Part 2 修复的 HT 对齐 bug，C 编译器 padding vs `__packed` 差异）
- 固件版本/配置对 HE STA 模式的支持程度
- HE 模式下需要不同的 SDIO header 格式或 HostDesc 字段

---

### 3.2 第二层：TX/RX 帧的过度拷贝（~20% 估计贡献）

**TX 路径 5 次拷贝**：

| # | 位置（文件:行） | 从 | 到 | 分配方式 |
|---|---|---|---|---|
| 1 | `router.rs` dispatch | smoltcp tx_buffer | `QueuedPacket` ([u8; 1500]) | 栈内固定 |
| 2 | `router.rs` EthernetDevice::send_to | `QueuedPacket` | Ethernet frame `Vec<u8>` | **堆分配** |
| 3 | `RdNetDriver` transmit | `Vec<u8>` | DMA buffer | 池分配 + write_with_cpu |
| 4 | `device.rs:232-244` AicTxQueue::submit | DMA buffer | `Vec<u8>` (.to_vec()) | **堆分配** |
| 5 | `tx.rs:381` build_data_frame | eth_frame slice | SDIO frame `Vec<u8>` (~1536B) | **堆分配** |

**RX 路径 7 次拷贝**：

| # | 位置 | 从 | 到 |
|---|---|---|---|
| 1 | `rx.rs` read_fifo_data | SDIO FIFO | `Vec<u8>` |
| 2 | `rx.rs` build_and_enqueue_eth_frame | 802.11 MPDU | Ethernet `Vec<u8>` |
| 3 | `device.rs` AicRxQueue::reclaim | data_queue frame | DMA buffer |
| 4 | `RdNetDriver` receive/prefetch | DMA buffer | VecRxBuffer (to_vec) |
| 5 | `router.rs` EthernetDevice::recv | Ethernet payload | DevicePacketBuffer |
| 6 | `router.rs` RX enqueue | DevicePacketBuffer | QueuedPacket |
| 7 | `router.rs` Router::poll | QueuedPacket | smoltcp rx_buffer |

**13.7Mbps 下的开销估算**：
- ~1140 个 1500B 帧/秒
- 每帧 TX 5 次拷贝 × 1140 = **每秒 5700 次 memcpy(~1500B)**
- 总 memcpy 带宽 ~8.5 MB/s（TX 侧），RX 侧更严重
- 每秒 ~3400 次堆 alloc/dealloc（仅 TX 侧 3 个 Vec）

**最冗余的拷贝**：第 4 次（`AicTxQueue::submit()` 中 `from_raw_parts().to_vec()`）——DMA buffer 已被网卡栈管理，aic8800 TX 队列应直接持有 DMA buffer 引用而非重新拷贝。

---

### 3.3 第三层：A-MPDU 聚合 — 驱动未发起 ADDBA（~10-15%）

**关键发现**：A-MPDU 聚合不仅"深度不够"，而是**驱动根本未发起 BlockAck 会话**。

**代码证据**：

1. **不发起 ADDBA 握手**：整个 driver 代码中无 `SM_ADDBA_REQ` 或类似消息的构造和发送
2. **不处理 TX CFM**（`tx.rs:432`）：
   ```rust
   // hostid = 0x8000_0001: bit31=1 请求固件 TX 确认
   hd[4..8].copy_from_slice(&0x8000_0001u32.to_le_bytes());
   ```
   固件的 TX CFM 回复（`SDIO_TYPE_CFG_DATA_CFM`）被 RX 线程收到后仅 debug 日志记录（`rx.rs:764-766`），不反馈到 TX 调度
3. **无速率控制回路**：MCS 选择完全由固件内部决定，host 无任何反馈

**澄清**：Part 2 文档中 `avg_ampdu_len 8-10×` 测量的是**下行（AP→STA）**聚合——即 StarryOS 做 SoftAP 时，固件对手机客户端方向发送的聚合帧。不是 StarryOS 作为 STA 发送上行流量时的聚合。

**影响**：无 A-MPDU 则每个 MPDU 独立封 PPDU，每次都需 PHY preamble（HT-mixed ~20µs）+ SIFS（16µs）+ BlockAck。对小帧（TCP ACK, ~64B），preamble 开销占比 >50%。

---

### 3.4 第四层：PIO + 单 SDIO 锁（~10%）

**PIO 路径**（`components/sdhci-cv1800/src/lib.rs:453-472`）：

```rust
fn pio_write(&self, buf: &[u8], block_size: u16, nblocks: u16) {
    let mut offset = 0;
    for _ in 0..nblocks {                         // 3 blocks for 1536B frame
        self.wait_buffer_write_ready()?;           // poll BUF_WR_READY (~200µs/block)
        for _ in 0..(block_size/4) {              // 128 iterations per block
            let word = u32::from_le_bytes(data);
            self.write::<u32>(SDHCI_BUFFER, word); // MMIO write to 0x20
            offset += 4;
        }
    }
}
```

**硬件 DMA 能力**：
- `SDHCI_CAPABILITIES` (0x40) bit19 = ADMA2 支持
- `SDHCI_DMA_ADDRESS` (0x00) 寄存器已定义但从未写入
- `SDHCI_SDMA_BOUNDARY_512K` 仅按规定写入 BLOCK_SIZE 寄存器（SDHCI 规范要求），不使能 DMA

**单 Mutex 问题**（`components/aic8800/src/fdrv/core/sdio_transport.rs:28`）：
```rust
pub struct SdioTransport {
    sdio: Arc<Mutex<dyn SdioHost>>,  // 所有 SDIO 操作共享一把锁
```

`write_fifo`（~650µs）、`read_fifo`、`read_flow_ctrl`（CMD52, ~5µs）、`read_byte` 全部经过这把锁。TX PIO 持锁期间 RX 完全阻塞，反之亦然。

**SDIO 时钟**：
- dev 分支：`HIGH_SPEED_CLOCK_HZ = 25_000_000`（注释称"50MHz 下大块 CMD53 不可靠"）
- Part 2 分支：提升到 50MHz + 从 vendor `sdhci-cv181x.c` 照抄的 PHY delay 配置（`MSHC_CTRL bit1 + PHY_CONFIG bit0 + TX_RX_DLY=0x01000100`）

---

### 3.5 第五层：流控策略保守 + CMD 优先中断批次（微观延迟，<5%）

**流控检查**（`tx.rs:255-268`）：

```rust
fn check_data_flow_control(transport) -> bool {
    for _ in 0..50 {                              // dev: 50 次; Part 2: 1 次
        let fc = transport.read_flow_ctrl_value(); // CMD52 (~5-10µs)
        if fc > DATA_FLOW_CTRL_THRESH { return true; }  // THRESH = 2
        runtime().yield_now();                    // 让出 CPU → 可能触发 50ms sched-rr 惩罚
    }
    false
}
```

- `DATA_FLOW_CTRL_THRESH = 2`：credits ≤2 即停止，保守
- 每发一帧前执行一次 CMD52 读 → ~5-10µs/帧
- 流控不足时 `yield_now()` → 可能触发 sched-rr 50ms 调度惩罚（**Part 1 的根因机制仍在起作用**）

**CMD 优先中断数据批处理**（`tx.rs:228-234`）：
```rust
while bus.tx.pktcnt > 0 {
    if batch_count >= TX_BATCH_LIMIT { break; }    // 64 帧上限
    if bus.cmd.pending_flag { break; }              // ★ CMD pending → 中断 DATA
    // ...
}
```

AP 模式下控制端口对账（`CONTROL_PORT_RECONCILE_MS = 50ms`）频繁产生 CMD，可能持续中断数据批处理。

**V3 芯片唤醒延迟**（`sdio_transport.rs:214-239`）：`wakeup()` 轮询 `SLEEP_REG` 最多 200 次 yield_now()，可能累积 ~50ms。

---

### 3.6 第六层：网络栈三层轮询模型（微观延迟，<5%）

三层独立轮询：

| 层级 | 任务名 | 空闲超时 | 唤醒方式 |
|---|---|---|---|
| 1 | `net-poll` | 100ms idle timeout | `NET_IRQ_NOTIFY`, `request_poll()`, socket wake |
| 2 | `{name}-rx` (per-device) | 10ms timeout | `rx_wake` via waker |
| 3 | `wifi-rx` + `wifi-tx` | 10ms/1ms kicker | waker + poll kicker 兜底 |

三层之间的帧搬运引入了前述的 7 次拷贝。`net-poll` 的 100ms idle 超时意味着真正空闲时的唤醒延迟上限为 100ms，但实际通过 IRQ notify 触发，延迟可控。

**smoltcp 配置**（`net/ax-net/Cargo.toml` + `consts.rs`）：
- smoltcp 0.13.1，启用 TCP/UDP/Raw/DHCP/DNS/IPv4/IPv6
- 禁用 IPv4 fragmentation/reassembly
- `SOCKET_BUFFER_SIZE = 64`（protocol packet buffer 槽数）
- TCP/UDP socket buffer：64KB
- `DEVICE_RX_QUEUE_SIZE = 256`，`DEVICE_TX_QUEUE_SIZE = 128`
- MTU = 1500

---

## 4. 优化方向（按优先级排序）

### P0：HE 数据通路修复（最关键，~60% gap）

**起点**：`components/aic8800/src/fdrv/protocol/config.rs` 中 `he_cap` 54 字节全零。

**步骤**：
1. 用 Wireshark 抓取 vendor Linux STA 连接时的完整 AssocReq/AssocResp
2. 提取 HE Capabilities IE + HE Operation IE 并逐字节对比
3. 排查 ME_CONFIG_REQ 中 HE 相关结构体的 C 编译器自然对齐 vs packed 的 padding 差异（HT 对齐 bug 的 HE 版本——HT 的对齐修复是 Part 2 的关键突破点，HE 极可能有同类问题）
4. 验证 HE 数据通路：ampdu != 0, DHCP 可通
5. 如 HE 数据通路仍然断，需排查固件版本对 HE STA 模式的支持

**预期收益**：65M → 115M PHY，TCP 上行 ~13.7M → ~22M（+60%）

**难度**：高（需要固件交互调试）| **风险**：中

---

### P1：消除冗余拷贝 + 预分配 buffer pool（~20% gap）

**最优先消除的拷贝**：

1. **TX 拷贝 #4**（`components/aic8800/src/fdrv/net/device.rs:232-244`）：
   - `AicTxQueue::submit()` 中 `from_raw_parts().to_vec()` → 改为直接持有 `DmaBuffer` 或 `Arc<DmaBuffer>` 引用
   - 或者将 `enqueue_data_frame` 的接口从 `Vec<u8>` 改为 `&[u8]`，延迟拷贝到 `build_data_frame` 一步

2. **TX 拷贝 #5**（`components/aic8800/src/fdrv/thread/tx.rs:381`）：
   - `build_data_frame()` 中 `vec![0u8; final_len]` → 使用栈数组或预分配池
   - 最大 SDIO 帧大小确定（1536B 对齐到 512 块边界 = 1536 或 2048B），可用 `[u8; 2048]` 栈数组

3. **RX 路径**：减少 `to_vec()` 调用，尽量传递 buffer 引用而非拷贝

**预期收益**：每秒减少 ~2000 次 heap alloc/dealloc + 减少 2 次 memcpy → 提升 5-10%

**难度**：低-中 | **风险**：低

---

### P2：ADMA2 DMA 传输（~10-15% gap）

**硬件依据**：CAPABILITIES bit19 = ADMA2 支持

**参考实现**：`milkv-duo/duo-buildroot-sdk` 的 `linux_5.10/drivers/mmc/host/cvitek/sdhci-cv181x.c`

**步骤**：
1. 在 `components/sdhci-cv1800/src/lib.rs` 中实现 ADMA2 descriptor 链
2. 为 `write_fifo`/`read_fifo` 添加 DMA 路径
   - 设置 `SDHCI_DMA_ADDRESS` 指向 descriptor 表
   - 启动 DMA 传输（设置 Transfer Mode 的 DMA Enable 位）
   - 通过 `NORM_INT_XFER_COMPLETE` 中断等待完成
3. DMA 传输期间释放 SDIO 锁（仅锁 descriptor 内存）

**前提依赖**：需要物理地址连续的 descriptor 内存，在 StarryOS 虚拟地址模型下需配合 `virt_to_phys` 使用

**预期收益**：消除 PIO 逐字等待，释放 CPU → 吞吐 +15-25%

**难度**：中-高 | **风险**：中（DMA 内存管理）

---

### P3：SDIO 锁细化（~5% gap）

**当前**：`Arc<Mutex<dyn SdioHost>>` 串行化全部 SDIO 操作

**方案**：
- CMD52（流控寄存器读，~5µs）与 CMD53（数据传输，~650µs）分离锁
- TX FIFO 写与 RX FIFO 读可部分并行（不同 register address 的 CMD53）
- 最简方案：至少将 `read_flow_ctrl_value()` 的锁与 `write_fifo` 分离

**预期收益**：减少 RX 在 TX 期间的阻塞，提升下行吞吐

**难度**：中 | **风险**：低

---

### P4：流控策略优化（<5% gap）

1. `DATA_FLOW_CTRL_THRESH` 从 2 降到 1（允许 credits=2 时继续发送）
2. 将 `check_data_flow_control()` 的 50 次重试改为 Part 2 分支版本的 1 次 + yield
3. 流控检查降频：从"每帧前"改为"每 N 帧一次"（如每 8 帧）

**难度**：低 | **风险**：低（固件有 buffer overflow 保护）

---

### P5：事件驱动唤醒替代周期性 kicker（<5% gap）

- RX/TX kicker（`sleep_ms(10)`/`sleep_ms(1)`）→ 纯事件驱动
- 确认 `PollSet` waker 机制可靠后，以更大的兜底间隔（如 100ms）保留 kicker 仅作为安全网

**难度**：低 | **风险**：低

---

### 不可行或低优先级的方向

| 方向 | 原因 |
|---|---|
| 40MHz 带宽 | Part 2 已验证，吞吐不变（瓶颈非空口时间） |
| rf_config 增益表 | Part 2 已验证导致关联失败 |
| 全栈 async/await 重写 | 热路径状态机分配开销抵消收益 |
| Rust `async fn` 替代 TX/RX poll task | 手工 `PollFn` + `block_on` 零分配更适合 1500fps 热路径 |
| 多帧 CMD53 拼包 | 需固件 `aicwf_sdio_aggr` 支持，DMA 之后才值得做 |

---

## 5. 总结表

| # | 根因 | 差距占比 | 难度 | 涉及关键文件 |
|---|---|---|---|---|
| 1 | HE 完全禁用 (54B all-zero capability) | **~60%** | 高 | `aic8800/src/fdrv/protocol/config.rs:102-129`, `lmac_msg.rs` |
| 2 | TX 5×拷贝 + 3×堆分配 / RX 7×拷贝 | ~20% | 低-中 | `device.rs:232`, `tx.rs:381`, `RdNetDriver` |
| 3 | A-MPDU 聚合未发起 (无 ADDBA, 无 TX CFM 处理) | ~10-15% | 中 | `config.rs`, `tx.rs` hostid, `rx.rs:764` |
| 4 | PIO + 单 SDIO 锁 (CAPABILITIES bit19=1 未使用) | ~10% | 中-高 | `sdhci-cv1800/src/lib.rs` |
| 5 | 流控保守 (THRESH=2, 50×重试, 每帧 CMD52) | <5% | 低 | `tx.rs:255`, `consts.rs:295` |
| 6 | 三层轮询 + 10ms kicker | <5% | 低 | `tx.rs:96-112`, `rx.rs:133`, ax-net router |

**最优路径**：P0（HE）+ P1（消除拷贝）是最独立且最高杠杆的两个方向，可并行推进。P0 解决 PHY 层的 60% gap，P1-P5 合力解决软件栈的 ~40% gap。HE + 消除冗余拷贝 + DMA 三者叠加，有望追平 Linux 33Mbps。

---

## 6. 验证方式

1. 同条件 iperf3 TCP 上行对比：iPhone 热点 ROS, ~30cm proximity，同位置同信号
2. 每次测试重采 vendor Linux baseline（RF 条件漂移）
3. 使用 `poll_int_status` 四段计时法（data_idle/cmd/buf_wr/xfer）验证微观效果
4. 建议在关键打点处预置 `ktracepoint::define_event_trace!` tracepoint（如 `wifi:xfer_done`, `wifi:tx_frame`），方便后续 eBPF 持续监测而不依赖函数符号稳定性

---

## 7. 相关文件索引

### SDHCI/SDIO 层
- `components/sdhci-cv1800/src/lib.rs` — SDHCI 控制器驱动，PIO write/read，CMD53 设置
- `components/sdhci-cv1800/src/regs.rs` — SDHCI 标准寄存器 + 中断位 + 时钟常量
- `components/sdhci-cv1800/src/hw_init.rs` — SoC 级 init：CRG/SYSCON/RTCSYS, PHY delay
- `components/sdhci-cv1800/src/runtime.rs` — `SdhciDelay` trait（yield_now/sleep 注入）
- `components/sdhci-cv1800/src/irq.rs` — SDIO CARD_INT ISR
- `components/sdio-host/src/lib.rs` — `SdioHost` trait 定义

### AIC8800 驱动核心
- `components/aic8800/src/runtime.rs` — `WifiRuntime` trait：spawn_poll_task, block_until, yield_now
- `components/aic8800/src/fdrv/thread/tx.rs` — TX 线程：帧构建，流控，CMD53 写入
- `components/aic8800/src/fdrv/thread/rx.rs` — RX 线程：FIFO 读取，帧分发，EAPOL 提取
- `components/aic8800/src/fdrv/core/sdio_transport.rs` — SDIO 操作封装，流控寄存器，V3 唤醒
- `components/aic8800/src/fdrv/protocol/config.rs` — `send_me_config_req()`: HT/HE 能力配置
- `components/aic8800/src/fdrv/protocol/lmac_msg.rs` — LMAC 消息结构体，HT/HE capability 尺寸
- `components/aic8800/src/fdrv/protocol/cmd.rs` — CMD 发送/CFM 等待框架
- `components/aic8800/src/fdrv/protocol/connection.rs` — `SM_CONNECT_REQ` 连接请求
- `components/aic8800/src/fdrv/net/device.rs` — `Net` trait 实现，TX/RX 队列
- `components/aic8800/src/fdrv/wifi/manager.rs` — 扫描/连接/密钥安装编排
- `components/aic8800/src/fdrv/wifi/api.rs` — `WifiClient` 高层 API
- `components/aic8800/src/fdrv/consts.rs` — TX_BATCH_LIMIT, DATA_FLOW_CTRL_THRESH 等

### 网络栈
- `net/ax-net/src/router.rs` — Router, RouteTable, DeviceHandle, 设备 TX/RX worker
- `net/ax-net/src/service.rs` — Service, net_poll_worker, DHCP client/server
- `net/ax-net/src/lib.rs` — init_network, SOCKET_SET, NET_POLL_WAKE
- `net/ax-net/src/consts.rs` — MTU, buffer sizes, queue capacities
- `net/ax-net/Cargo.toml` — smoltcp features
- `os/arceos/modules/axruntime/src/wifi_glue.rs` — ArceOS `WifiRuntime` 实现
- `drivers/ax-driver/src/net/aic8800.rs` — FDT probe，设备注册到 ax-net

### 配置与测试
- `os/StarryOS/kernel/Cargo.toml` — sched-rr (MAX_TIME_SLICE=5, TICKS_PER_SEC=100 → 50ms)
- `test-suit/starryos/board-licheerv-nano-sg2002/` — SG2002 board 测试配置
- `apps/starry/picoclaw-cli/WIFI_SWITCH_DEMO.md` — wifi_switch 用户态工具
- `os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml` — WiFi board config
