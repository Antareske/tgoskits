# SG2002 WiFi 除了忙等外还改了什么？—— Part 1/2 全部修改详解

> 教学文档：逐一解释 `sg2002-wifi.md` 中所有改动的内容、原因和效果。
> 原始分支：`test/verify-net-wakeup-fix`（不在当前 `sg2002/wifi-irq` 分支）。

---

## 全景：从 0.2 Mbps 到 13.7 Mbps 的完整旅程

```
Part 1 起点 (legacy-g)      0.2 Mbps    ← 基本不可用
  │
  ├─ ① 3ms 忙等 (Part 1)    ~10 Mbps    ← 50× 提升，根因修复
  ├─ ② 关网络 Debug 日志     稳定到 10.6
  │
  ├─ ③ HT 结构体对齐修复     12.7 Mbps   ← 根因 bug，解锁 A-MPDU 聚合
  ├─ ④ TX/RX kicker 1ms
  │
  ├─ ⑤ SDIO 50MHz + PHY     13.7 Mbps   ← 系统层突破，下行翻倍
  ├─ ⑥ 流控空转修复
  └─ ⑦ 日志级别 Error
```

其中 ①（忙等）你已经了解。下面逐一解释其余 6 项。

---

## ② 关闭网络栈 Debug 日志

### 改了什么

把 board toml 中日志级别从 `Debug`/`Info` 调成 `Warn`，具体关掉的是 smoltcp 和 ax_net 的 Debug 输出。

### 为什么

开了 Debug 后，每个网络包的处理都会在 **115200 baud 同步 UART** 上打一行日志。按 115200 bps ≈ 每字节 ~87µs 算，一行几十字节的 Debug 日志就要写 **几毫秒**。同步 UART 写是阻塞的——CPU 等着 FIFO 排空——这几毫秒里 TCP 协议栈、WiFi 驱动全被卡住。

实测：开着 Debug 时上行在 2.6-8 Mbps 剧烈波动，关掉后稳定到 10.2-10.6 Mbps。

### 一句话

**UART 同步打印是最慢的 I/O 之一**，在热路径上每包打日志会把网络性能拖垮。

---

## ③ HT 结构体对齐修复（根因 bug）

### 改了什么

三个常量修正：

| 常量 | 修复前（packed 尺寸） | 修复后（自然对齐尺寸） |
|------|----------------------|----------------------|
| `MAC_HT_CAPABILITY_SIZE` | 26 | **32** |
| `MAC_HE_CAPABILITY_SIZE` | 54 | **56** |
| `ME_CONFIG_REQ_SIZE` | 102 | **112** |

这些常量在 `components/aic8800/src/fdrv/protocol/lmac_msg.rs` 中，用于构造发送给固件的 `ME_CONFIG_REQ` 消息。

### 为什么

这是整个 Part 2 最重要的发现——**一个结构体对齐 bug 让 WiFi 一直以 802.11g 传统模式连接**。

事情的脉络：

1. **固件那边的结构体不是 `__packed` 的**。C 编译器按自然对齐规则给结构体成员之间插 padding。比如一个 `u32` 字段必须放在 4 字节对齐的地址上，如果前面只有 26 字节，编译器会塞 2 字节 padding 到 28，然后放这个 u32。

2. **我们的 Rust 侧按 packed 尺寸算**。Rust 的 `#[repr(C)]` 虽然也做自然对齐，但关键是：我们构造消息时给 `ht_supp`（一个标志位，告诉固件"我支持 HT"）写的偏移是按 packed 尺寸（26）算的，固件读的偏移是按自然对齐（32）算的。

3. **固件读到了 0**。`ht_supp` 写在 `param[95]`，固件在 `param[103]` 读——读到的是未初始化的 0。

4. **固件以为不支持 HT** → 不发 HT Capabilities IE → 关联时退化成 802.11g 传统模式。

5. **802.11g 不支持 A-MPDU 聚合**（BlockAck 机制）。

6. **没有 A-MPDU 聚合，每发一个 MPDU 都要单独封 PPDU**，每次都走一遍 PHY preamble（~20µs）+ SIFS（16µs）+ BlockAck。对小帧（如 TCP ACK），preamble 开销占比 >50%。

### 效果

修复前 `avg_ampdu_len` 恒为 65536（这个值是 1.0 按 16.16 定点数编码，意思是聚合深度 = 1，即无聚合）。修复后 `avg_ampdu_len = 300,000-700,000`（4.6-10.7× 聚合）。**AssocReq 包中第一次出现 HT IE（0x2D）**——这意味着之前一直没通告 HT 能力。

TCP 上行从 ~10M 到 12.7M（+27%）。

### 一句话

**固件结构体有 C 编译器自动插入的 padding，我们按 packed 尺寸算偏移，导致 HT 能力标志写到错误位置，固件读成 0 → 以为不支持 HT → 退回 802.11g → 无聚合 → 吞吐被空口效率卡死。**

这是那种"一行改动，性能翻倍"的经典案例——不是拼凑优化参数，而是修了一个让整个 HT 协议栈完全不工作的偏移计算错误。

---

## ④ TX/RX kicker 10ms → 1ms

### 改了什么

WiFi 驱动的 TX 和 RX 各有独立 poll task，它们靠事件驱动（wake）来工作。但事件驱动可能丢唤醒（`PollSet` 无 sticky 位——如果一个 IRQ 在 task 检查完队列之后、调用 `wait` 之前到达，就没有人会再唤醒它），所以需要一个**兜底 kicker**：定期醒来扫一眼有没有漏掉的工作。

这个 kicker 的周期从 10ms 改成 1ms。

### 为什么

ping RTT 的 min/avg/max 是 10.8/134/2078 ms（本地 WiFi 应该在 1-5ms），而且 RTT 的众数死锁在 ~33ms——说明 poll task 经常在等 kicker 兜底唤醒，而不是被事件及时唤醒。

10ms → 1ms 后，最坏情况下 poll task 只白等 1ms 就能发现漏掉的工作。

### 效果

TCP 上行 10.6 → 12.7M（**+20%**）。RTT 大幅改善。

### 一句话

**事件驱动偶尔丢唤醒，poll task 靠定时 kicker 兜底。10ms 太慢，1ms 够快——在"丢失等待"和"CPU 空转"之间取一个更紧的平衡。**

---

## ⑤ SDIO 50MHz + PHY delay 配置（系统层突破）

### 改了什么

三个改动：

1. **SDIO 时钟翻倍**：`HIGH_SPEED_CLOCK_HZ` 从 25MHz → 50MHz
2. **SoC PHY delay 寄存器配置**（从 vendor Linux 驱动照抄）：
   - `VENDOR_MSHC_CTRL` (0x200) |= BIT(1)
   - `VENDOR_PHY_CONFIG` (0x24C) |= BIT(0)
   - `VENDOR_PHY_TX_RX_DLY` (0x240) = `0x01000100`

### 为什么

**时钟**：SDIO 4-bit 模式下，25MHz 的理论带宽是 12.5 MB/s，50MHz 是 25 MB/s。PIO 每字节都要 CPU 执行 MMIO 指令搬运，时钟翻倍意味着同样的 CPU 开销能搬两倍的数据。

**PHY delay**：这才是关键。代码里有一条注释说"50MHz 下大块 CMD53 不可靠"——原因是 SDIO 总线跑 50MHz 时，时钟和数据线之间的时序关系非常敏感。PCB 走线长度差异、芯片内部延迟都会导致数据和时钟的边沿错位（建立/保持时间违例），表现为 DAT CRC error。

vendor 的 Linux 驱动（`sdhci-cv181x.c`）在 SDIO reset 时会写一组 PHY delay 寄存器，微调时钟和数据的相位关系，让高速信号的眼图重新张开。我们的驱动之前从不写这些寄存器（全是 0）——相当于 50MHz 下信号质量不够，所以被注释成"不可靠"。

### 踩坑

第一次没有照抄 vendor 的精确值，自己猜了 bit8/bit9 → 直接 DAT CRC error。**必须精确照抄 vendor 序列。**

### 效果

- 下行从 ~7M → 13.7M（**+96%，几乎翻倍**）
- 4 路并发从 ~12M → 18.9M
- 上行从 12.7M → 13.7M
- WiFi 连接稳定，无 CRC error

### 一句话

**SDIO 时钟翻倍 + 照抄 vendor 的 PHY 时序校准寄存器 = 总线带宽翻倍，下行直接翻番。这是修改最少但效果最炸裂的一项。**

---

## ⑥ 流控空转修复

### 改了什么

`check_data_flow_control()` 逻辑从：

```rust
// 旧：最多 50 次重试，每次都读 SDIO 寄存器 + yield
for _ in 0..50 {
    let fc = transport.read_flow_ctrl_value();  // CMD52 ~5-10µs
    if fc > DATA_FLOW_CTRL_THRESH { return true; }
    runtime().yield_now();
}
```

改为：

```rust
// 新：读 1 次，不够就 yield 一次，直接返回
let fc = transport.read_flow_ctrl_value();
if fc > DATA_FLOW_CTRL_THRESH { return true; }
runtime().yield_now();
false
```

### 为什么

50MHz SDIO 让 host 写数据的速度更快了——灌满 firmware TX buffer 的速度比 radio 排空的速度快得多。结果是流控（flow control）频繁触发——firmware 的 credit 很快掉到 ≤2（`DATA_FLOW_CTRL_THRESH`）。

旧代码在流控触发时做 **50 次空转**：每次读 CMD52 寄存器（~5-10µs），发现 credit 还是不够，然后 yield。50 次 × 5-10µs = 250-500µs 的 SDIO 总线被白白占用。在 TX 线程的计时窗口里，流控读（fcrd）占了 **40-54%** 的时间——也就是说 **TX 线程一半的时间在徒劳地读流控寄存器**。吞吐反降到 6M。

### 效果

fcrd 占比从 40-54% → **0.4%**。吞吐从 6M → 12.4M 恢复。

### 一句话

**50MHz 让固件 buffer 更快被灌满 → 流控更频繁触发 → 50 次空转让 TX 线程一半时间在做无用功。改成读 1 次 + yield 就解决了。**

---

## ⑦ 日志级别 Warn → Error

与 ② 同理——进一步减少同步 UART 输出。生产配置开到 Error 级别。

---

## 综合来看：这些改动的层次结构

把这些改动按"修改的层"分类：

```
应用层
  ├─ ②⑦ 日志级别            ← 减少同步 I/O 扰动

WiFi 协议层
  ├─ ③ HT 对齐修复           ← 让 802.11n/A-MPDU 真正工作
  └─ ④ kicker 1ms            ← 减少 poll task 延迟

SDIO 总线层
  ├─ ① 3ms 忙等 (Part 1)     ← 消除 50ms 调度惩罚
  ├─ ⑤ 50MHz + PHY delay     ← 翻倍总线带宽
  └─ ⑥ 流控空转修复          ← 消除高频流控查询的开销
```

每一层解决一类问题，互不替代。这也是为什么 **Part 1 的忙等修复（①）是必要的但不是充分的**——它解决了"每次传输等 48ms"的调度问题，但如果不修 HT 对齐（③），HT 根本不工作；如果不提 SDIO 时钟（⑤），总线带宽就是上限。

---

## 当前 `sg2002/wifi-irq` 分支缺了什么

当前分支只包含中断驱动的重构（XFER_COMPLETE 中断唤醒），对应 Part 1 忙等修复的"中断替代版本"。上述 6 项（②-⑦）均不在此分支。

| 改动 | Part 1/2 分支 | 当前 wifi-irq 分支 |
|------|:---:|:---:|
| 忙等/中断等待 | 3ms 忙等 | XFER IRQ + 10ms 安全网 |
| 关 Debug 日志 | ✅ | ❌ |
| HT 对齐修复 | ✅ | ❌ |
| kicker 1ms | ✅ | ❌ |
| SDIO 50MHz + PHY | ✅ | ❌（仍 25MHz） |
| 流控 1× 重试 | ✅ | ❌（仍 50×） |
| 日志 Error | ✅ | ❌ |

**这就是为什么当前分支 TX 只有 0.85 Mbps 而 Part 1 有 10 Mbps。** 忙等→中断的方向是正确的，但还需要把上述 6 项合入才能真正发挥作用。

---

## 推荐合入顺序

1. **②⑦ 日志级别**——零风险，独立改动，直接减少 UART 抖动
2. **⑤ SDIO 50MHz + PHY delay**——需要照抄 vendor 序列，独立改动，总线带宽翻倍
3. **③ HT 对齐修复**——独立于等待策略，解锁 A-MPDU 聚合
4. **⑥ 流控空转修复**——依赖 ⑤（50MHz 才暴露），但改动独立
5. **④ kicker 1ms**——独立改动，减少 poll 延迟

每一项都可以独立提交、独立验证。
