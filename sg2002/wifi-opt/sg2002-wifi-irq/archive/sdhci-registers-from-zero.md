# SDHCI 寄存器：从零开始

这份文档写给没有任何硬件编程背景的人。会用日常比喻解释每一个概念，然后逐步进入 SDHCI 控制器的具体寄存器。

---

## 第 1 节：前置概念

### 1.1 什么是"位"（bit）

计算机里所有信息最终都是 0 和 1。一个"位"就是一个只能存 0 或 1 的开关。

8 个位组成一个"字节"（byte），16 个位组成一个"半字"（half-word），32 个位组成一个"字"（word）。

在驱动代码中，你会看到这样的类型：

```rust
self.read::<u8>(addr)    // 读 8 位（1 字节）
self.read::<u16>(addr)   // 读 16 位（2 字节）
self.read::<u32>(addr)   // 读 32 位（4 字节）
```

### 1.2 什么是"寄存器"（register）

**寄存器是硬件里的一小块存储空间**。

想象一个快递柜。柜子有 100 个格子，每个格子能放一张纸条。CPU 和硬件通过往格子里放纸条、取纸条来通信：

- CPU 往 3 号格子放一张纸条写"25000000"→ 硬件读到后把 SDIO 时钟设为 25MHz
- 硬件在传输完成后往 8 号格子放一张纸条写"1"→ CPU 读到后知道传输完成了

这里的"格子"就是寄存器，格子的编号就是地址，纸条上的数字就是寄存器的值。

**寄存器和普通内存的区别**：

普通内存（RAM）里你写 42，下次读就是 42。但寄存器连接的是硬件电路——你写 0 可能"关闭时钟"，写 1 可能"打开时钟"；你读出来的值可能反映"现在总线上有没有数据"。

所以寄存器的本质是：**CPU 和硬件电路之间的通信接口**。

### 1.3 什么是 MMIO

MMIO = Memory-Mapped I/O，中文叫"内存映射 I/O"。

CPU 只有一种方式和外部的硬件通信：读写内存地址。

但硬件不是内存。怎么让 CPU 用 L 读写内存指令去控制硬件？答案是把硬件的寄存器"映射"到内存地址空间。

**比喻**：你只有一个快递柜系统。本来 1~50 号格子对应真正的储物箱（内存）。51 号往后的格子没有实体储物箱，而是连接到隔壁大楼（硬件）。你往 51 号格子放纸条，纸条实际被送到了隔壁大楼的传达室。

在我们的 SG2002 芯片上：

| CPS 看到的地址 | 实际是什么 | Rust 中的体现 |
|:---|:---|:---|
| `0x8000_0000` | DDR 内存（真正的 RAM） | 普通变量 |
| `0x0432_0000` | SDHCI 控制器寄存器 | `self.read::<u16>(0x30)` |

**关键**：CPU 完全不知道区别。`ldr` 指令加载地址 `0x8000_0000` 读到的是内存数据，加载 `0x0432_0030` 读到的是 SDHCI 的中断状态寄存器。对 CPU 来说都是"读某个地址"，但背后的物理电路完全不同。

在 Rust 代码中：

```rust
// read_volatile: 读某个内存地址的值
fn mmio_read<T: Copy>(addr: usize) -> T {
    unsafe { read_volatile(addr as *const T) }
}

// 读 SDHCI 中断状态寄存器（基地址 0x04320000 + 偏移 0x30）
let status = mmio_read::<u16>(0x04320000 + 0x30);
```

`read_volatile` 告诉编译器："每次都要真的读，不要自作聪明缓存上次的值"。因为寄存器的值会自己变（硬件会改），编译器不能假设"上次读是 0 这次还是 0"。

### 1.4 寄存器地址 = 基地址 + 偏移

SDHCI 控制器有一组寄存器，它们的地址是连续的。驱动里存一个基地址，然后用偏移量访问不同寄存器：

```rust
pub struct CviSdhci {
    base: usize,  // 基地址，例如 0x04320000
}

// 读偏移 0x30 处的寄存器 = 读地址 0x04320000 + 0x30 = 0x04320030
fn read<T: Copy>(&self, off: u32) -> T {
    mmio_read::<T>(self.base + off as usize)
}
```

代码里的常量就是偏移量：

```rust
const SDHCI_INT_STATUS_NORM: u32 = 0x30;  // 这个寄存器在基地址 + 0x30 的位置
const SDHCI_CLOCK_CONTROL: u32 = 0x2C;    // 这个在基地址 + 0x2C
```

### 1.5 位移和掩码——怎么操作寄存器里的某一个位

寄存器通常是 16 位或 32 位的，但我们常常只想读写其中的某一位或某几位。这就需要位运算。

**读某一位**：用 `&`（按位与）检查。

```rust
// 检查 bit 1 是否为 1
let norm: u16 = read(0x30);           // 假设读到 0b0000_0000_0000_0010（bit 1 = 1）
let xfer_complete_mask: u16 = 1 << 1; // 0b0000_0000_0000_0010（只有 bit 1 是 1）
if norm & xfer_complete_mask != 0 {
    // bit 1 是 1！传输完成了！
}

// 写成常量的形式：
const NORM_INT_XFER_COMPLETE: u16 = 1 << 1;  // bit 1
if norm & NORM_INT_XFER_COMPLETE != 0 { ... }
```

`1 << 1` 的意思是"把数字 1 左移 1 位"：`0b...0001` → `0b...0010`。

**写某一位**：用 `|`（按位或）来设 1。

```rust
// 把 bit 1 设为 1（保持其他位不变）
let cur = read(0x38);                      // 假设读到 0b0000_0001_0000_0000
let new = cur | (1 << 1);                  // 0b0000_0001_0000_0000 | 0b0000_0000_0000_0010
                                           // = 0b0000_0001_0000_0010
write(0x38, new);
```

**清某一位**：用 `& !(...)` 来设 0。

```rust
// 把 bit 1 设为 0（保持其他位不变）
let cur = read(0x38);                      // 假设读到 0b0000_0001_0000_0010
let new = cur & !(1 << 1);                 // 0b...0010 & !0b...0010 = 0b...0010 & 0b...1101
                                           // = 0b0000_0001_0000_0000
write(0x38, new);
```

---

## 第 2 节：中断系统是怎么工作的

### 2.1 生活中断的比喻

你在等快递。

普通方式（轮询）：每 30 秒去门口看一眼快递到了没。看到 100 次，99 次白看。这就是 `yield_now()` 方式——不断让出 CPU，让快递等 50ms 再被调回来查看。

中断方式：快递到了，快递员按门铃。你听到铃声去开门。等待期间你可以干别的事（睡觉、做饭）。这就是我们要实现的方式——硬件完成时触发中断，CPU 立刻处理。

### 2.2 硬件中断怎么发生的

SDHCI 控制器内部有一套电路，它监控 SDIO 总线的状态。当"传输完成"这个事件发生时：

1. 电路把**状态寄存器**（STATUS）的第 1 位从 0 翻成 1
2. 电路检查**信号使能寄存器**（SIG_EN）的第 1 位——你允许我发中断吗？
3. 如果允许，控制器拉高一根物理引脚（一根连接到 PLIC 的导线）
4. PLIC 看到这根线变高了，决策路由给 CPU 的哪个核心
5. CPU 收到中断，暂停当前执行的代码，跳转到 ISR（中断服务程序）

### 2.3 为什么要有两个使能层？STATUS ENABLE 和 SIGNAL ENABLE 的区别

这就像你家的门铃系统：

```
第 1 层：日志本                        ← STATUS 寄存器
  快递来了 → 在日志本上记一笔"某月某日快递到达"
  不管你有没有要求通知，日志本上都有记录

第 2 层：通知方式过滤器                ← STATUS ENABLE
  日志本上可以记"快递到达"这一项吗？
  勾上 → 到达事件会被记入日志本
  不勾 → 发生的事日志本上没记录（但对应 STATUS 位也不会变）

第 3 层：门铃开关                     ← SIGNAL ENABLE
  日志上有记录后，你希望门铃响吗？
  开 → 门铃会响 → ISR 被调用
  关 → 只是日志上记了，门铃不响，你以后自己翻日志看（轮询）
```

**为什么不在 ISR 里处理所有中断？**

ISR 运行在"中断上下文"——一个非常受限的环境。ISR 里不能睡觉、不能等锁、不能分配内存。因为 ISR 阻塞了所有同级和低级中断，它必须尽可能快。

所以我们的设计是：
- **CARD_INT**：必须在 ISR 里处理（只需设置一个标志 + 唤醒线程，很快）
- **XFER_COMPLETE**：在 ISR 里只 mask 信号 + 唤醒一个等待的线程（也很快）
- **CMD_COMPLETE、BUF_RD_READY、BUF_WR_READY**：它们的发生非常频繁（每 4 字节一次），如果在 ISR 里处理，ISR 会太慢。所以它们的 SIG_EN 位永远是 0——只在 STATUS 里记录，由线程轮询检查

---

## 第 3 节：中断相关寄存器详解

### 3.1 寄存器地图

SDHCI 标准定义了这些寄存器的地址（偏移量，从基地址算）。我们关心的是中断相关的一组：

```
地址偏移    名称                    位宽    读写方式
0x30       INT_STATUS_NORM          16     读正常 / 写 1 清除
0x32       INT_STATUS_ERR           16     读正常 / 写 1 清除
0x34       NORM_INT_STS_EN          16     读 / 写
0x36       ERR_INT_STS_EN           16     读 / 写
0x38       NORM_INT_SIG_EN          16     读 / 写
0x3A       ERR_INT_SIG_EN           16     读 / 写
```

六边形，两列三行：

```
                  NORMAL（正常事件）           ERROR（错误事件）
STATUS           0x30 INT_STATUS_NORM         0x32 INT_STATUS_ERR
STATUS ENABLE    0x34 NORM_INT_STS_EN          0x36 ERR_INT_STS_EN
SIGNAL ENABLE    0x38 NORM_INT_SIG_EN          0x3A ERR_INT_SIG_EN
```

### 3.2 三级漏斗

把这三行看成对硬件事件的三层过滤：

```
    硬件事件发生
    （比如："传输完成了！"）
           │
           ▼
   ┌────────────────────────────┐
   │ ① STS_EN（状态使能）       │  ← 第一道门：这件事我关心吗？值得记吗？
   │   bit=1：允许进入 STATUS   │     通常全开，所有事件都先记下来再说
   │   bit=0：丢弃，不进 STATUS  │
   └────────────┬───────────────┘
                │ 通过后
                ▼
   ┌────────────────────────────┐
   │ ② STATUS（状态寄存器）     │  ← 锁存：硬件置 1，保持到软件写 1 清除
   │   bit=1：事件发生过了       │     像留言板上的便利贴，会一直贴在那
   │   软件写 1 来揭掉便利贴      │     直到你去揭掉（W1C）
   └────────────┬───────────────┘
                │ STATUS 位 = 1
                ▼
   ┌────────────────────────────┐
   │ ③ SIG_EN（信号使能）       │  ← 第二道门：这件事值得打断 CPU 吗？
   │   bit=1：允许产生中断       │     选择性开，只让非常紧急的事触发 ISR
   │   bit=0：不产生中断          │
   └────────────┬───────────────┘
                │ STATUS=1 且 SIG_EN=1
                ▼
          ┌──────────┐
          │ 拉高 IRQ  │  → PLIC → CPU 跳转 ISR
          └──────────┘
   ```

   **关键理解**：STATUS 位和 SIG_EN 位是独立的。

   - 你可以 SIG_EN=0（不触发中断），但 STATUS 仍然会记录事件。你以后通过轮询 STATUS 来发现事件。
   - 如果 SIG_EN=1，只要 STATUS 位还是 1，中断线就保持高电平。ISR 返回后如果 STATUS 还是 1，CPU 会立刻再次进入 ISR（中断风暴）。这就是为什么 ISR 必须 mask SIG_EN。

   ### 3.3 逐位说明：NORM_INT_STATUS 的每一位

   以偏移 0x30 的 `INT_STATUS_NORM` 为例。这是一个 16 位寄存器，每一位代表一个事件：

   ```
   bit 15 14 13 12 11 10  9  8  7  6  5  4  3  2  1  0
        │  │  │  │  │  │  │  │  │  │  │  │  │  │  │  │
        │  │  │  │  │  │  │  │  │  │  │  │  │  │  │  └─ bit  0: CMD_COMPLETE     命令完成
        │  │  │  │  │  │  │  │  │  │  │  │  │  │  └──── bit  1: XFER_COMPLETE    传输完成
        │  │  │  │  │  │  │  │  │  │  │  │  │  └─────── bit  2: (卡插入/移除)
        │  │  │  │  │  │  │  │  │  │  │  │  └────────── bit  3: (卡中断)
        │  │  │  │  │  │  │  │  │  │  │  └───────────── bit  4: BUF_WR_READY     缓冲区可写
        │  │  │  │  │  │  │  │  │  │  └──────────────── bit  5: BUF_RD_READY     缓冲区可读
        │  │  │  │  │  │  │  │  │  └─────────────────── bit  6: (DMA 中断)
        │  │  │  │  │  │  │  │  └────────────────────── bit  7: (块间空隙事件)
        │  │  │  │  │  │  │  └───────────────────────── bit  8: CARD_INT         卡中断
        │  │  │  │  │  │  └──────────────────────────── bit  9: (重调事件)
        │  │  │  │  │  └─────────────────────────────── bit 10: (调优错误)
        │  │  │  │  └────────────────────────────────── bit 11: (主机错误)
        │  │  │  └───────────────────────────────────── bit 12~14: (保留)
        │  │  └──────────────────────────────────────── bit 15: ERROR             汇总错误
   ```

   **我们关心的 6 位**：

   | 位 | 代码中的常量 | 硬件含义 | 谁置位 | 谁清除 |
   |:---|:---|:---|:---|:---|
   | bit 0 | `NORM_INT_CMD_COMPLETE` | 命令已发送并收到响应 | 硬件 | 软件 W1C |
   | bit 1 | `NORM_INT_XFER_COMPLETE` | 数据阶段已完成（FIFO 的数据已通过总线发给芯片） | 硬件 | 软件 W1C |
   | bit 4 | `NORM_INT_BUF_WR_READY` | FIFO 有空间，可以写入下一个字（4 字节） | 硬件 | 软件 W1C |
   | bit 5 | `NORM_INT_BUF_RD_READY` | FIFO 有数据，可以读出下一个字 | 硬件 | 软件 W1C |
   | bit 8 | `NORM_INT_CARD_INT` | WiFi 芯片对 CPU 说"我有事找你" | WiFi 芯片通过 SDIO 总线 | 软件 W1C |
   | bit 15 | `NORM_INT_ERROR` | 上面 bit 0~14 中任一错误发生过 | 硬件 | 软件 W1C |

   ### 3.4 什么是"锁存"和"W1C"

   SDHCI 的 STATUS 寄存器有两种行为：

   **锁存型（sticky）**：硬件置位后，不论硬件状态如何变化，位保持 1，直到软件显式清除。

   比喻：便利贴。快递员把便利贴贴在门上（硬件置 1），便利贴会一直贴在那，不管你有没有开门拿快递，直到你伸手揭掉便利贴（软件 W1C 清除）。如果你不揭，快递员不能"取消"这个便利贴——事件已经发生了。

   **W1C（Write-1-to-Clear）**：清除方式是写 1 到该位。

   听起来反直觉——"我想把 bit 1 清成 0，但我写 1？"

   实际上：STATUS 寄存器不是普通的存储单元。写 1 到 bit 1 被硬件电路解释为"清除 bit 1"的命令：

   ```rust
   // 清除 bit 1（XFER_COMPLETE）：写 bit 1 = 1
   self.write::<u16>(SDHCI_INT_STATUS_NORM, 1 << 1);

   // 注意！如果你写：
   self.write::<u16>(SDHCI_INT_STATUS_NORM, 0xFFFF);
   // 你清除了所有 16 位！
   // 如果你恰好把 CARD_INT（bit 8）也清了，WiFi 芯片的中断就丢了。
   ```

   **W1C 的关键风险**：写整个寄存器会把所有置 1 的位全部清除。这就是为什么我们修复后改为"选择性清除"：

   ```rust
   // ❌ 原来：全量清除，可能误清 CARD_INT
   self.write::<u16>(SDHCI_INT_STATUS_NORM, sts);

   // ✅ 修复后：只清除错误位 + 当前等待位
   self.write::<u16>(SDHCI_INT_STATUS_NORM, NORM_INT_ERROR | bit);
   ```

   ### 3.5 逐位追踪：一次传输中中断位的变化

   以 TX 写 512 字节数据帧为例，展示 STATUS 寄存器各位随时间的变迁。

   **阶段 1：CMD53 命令阶段**

   ```
   CPU 写 COMMAND 寄存器（发 CMD53）
     │
     ▼
   硬件开始在 SDIO 总线上发 CMD53
     │
     ▼
   WiFi 芯片响应 CMD53
     │
     ▼
   硬件置位 INT_STATUS[bit 0] = CMD_COMPLETE = 1
     │
     ▼
   CPU 轮询看到 bit 0 = 1 → W1C 清除 bit 0
     │
     ▼
   进入数据阶段
   ```

   **阶段 2：PIO 写入 FIFO**

   ```
   CPU 准备写第一个 4 字节字到 BUFFER 寄存器
     │ 先检查：FIFO 有空间吗？
     ▼
   读 INT_STATUS[bit 4] = BUF_WR_READY
     ├─ = 1：FIFO 有空间，写 BUFFER 寄存器
     │       写完后硬件自动把 BUF_WR_READY 清 0（数据阶段行为不同）
     │       硬件把数据从 FIFO 搬到总线 → FIFO 又有空间 → 重新置 1
     │
     └─ = 0：FIFO 满，等硬件把数据发走
          每 ~几微秒 FIFO 就又有空间 → bit 4 重新变 1

   这个过程重复 512÷4=128 次（每 4 字节一个字，一次 512 字节块）
   所以 BUF_WR_READY 会在 0 和 1 之间反复翻转 128 次
   ```

   **阶段 3：等待传输完成**

   ```
   所有 128 个字都写完了。现在数据在 SDHCI 控制器的 FIFO 里，
   正在通过 SDIO 总线发往 WiFi 芯片。

   CPU 调用 wait_transfer_complete() → poll_int_status(bit 1)

   轮询 bit 1 = XFER_COMPLETE：
     ├─ = 0：还在发...（~200µs）
     │       Phase 1 自旋 1000 次（~50µs）
     │       Phase 2 block_timeout
     │
     └─ = 1：FIFO 清空了！WiFi 芯片确认收到了全部数据！
            → W1C 清除 bit 1
            → 返回 Ok
   ```

   ### 3.6 SIG_EN 的动态变化

   这是一个重要的细节：**SIG_EN 的值在运行时会变**。

   ```
   时间线：
   开机初始化 →
     SIG_EN = CARD_INT 位 = 1      ← 始终开着，有 CARD_INT 就触发 ISR
              所有其他位 = 0        ← 不开，用轮询检查

   TX 线程进入 poll_int_status Phase 2（等待 XFER_COMPLETE）→
     unmask_xfer_complete_signal()
     SIG_EN = CARD_INT 位 = 1
            + XFER_COMPLETE 位 = 1  ← 临时打开，等着硬件给信号

   硬件完成传输 →
     INT_STATUS[bit 1] = 1
     SIG_EN[bit 1] = 1
     → STATUS & SIG_EN = 1 → 中断触发！

   ISR 进入 →
     读到 INT_STATUS[bit 1] = 1
     mask XFER_COMPLETE 信号：
       SIG_EN = CARD_INT 位 = 1
              + XFER_COMPLETE 位 = 0  ← 关掉！防止 ISR 返回后立刻再次触发

   ISR 返回 →
     TX 线程被唤醒
     读 INT_STATUS[bit 1] → 1（锁存位还在！ISR 没清它）
     W1C 清除 bit 1

   下次 TX 线程进入 poll_int_status Phase 2 →
     unmask_xfer_complete_signal()
     SIG_EN = CARD_INT 位 = 1
            + XFER_COMPLETE 位 = 1  ← 重新打开
   ```

   这就是为什么代码里每轮循环都调用 `unmask_xfer_complete_signal()`——因为 ISR 每次都 mask 掉它。

   ### 3.7 另一个视角：寄存器的物理意义

   总结一下这六个寄存器到底在硬件层面"连着什么"：

   | 寄存器 | 物理连接 |
   |:---|:---|
   | INT_STATUS_NORM | 16 根"事件发生"信号线，硬件可以把任何一根拉高。软件写 1 是拉低命令 |
   | INT_STATUS_ERR | 同上，但是错误类事件 |
   | NORM_INT_STS_EN | 16 个 AND 门：STATUS 线 AND STS_EN 线 → 结果连到 STATUS 的输入 |
   | ERR_INT_STS_EN | 同上 |
   | NORM_INT_SIG_EN | 16 个 AND 门：STATUS 线 AND SIG_EN 线 → 结果连到 IRQ 生成电路 |
   | ERR_INT_SIG_EN | 同上，输出也连到同一个 IRQ 生成电路 |

   最终的 IRQ 信号（连到 PLIC 的那根线）= `(NORM_STATUS & NORM_SIG_EN) != 0 || (ERR_STATUS & ERR_SIG_EN) != 0`——即任何一个被使能的位被置位，IRQ 就拉高。

   ---

   ## 第 4 节：代码如何操作寄存器

   ### 4.1 读改写（RMW）模式

   修改寄存器中某一位的经典三步：

   ```rust
   // 启用 XFER_COMPLETE 信号（SIG_EN 偏移 0x38）
   pub fn unmask_xfer_complete_signal() {
       let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
       if base == 0 { return; }
       let addr = base + SDHCI_NORM_INT_SIG_EN as usize;  // 0x04320000 + 0x38
       let cur = mmio_read::<u16>(addr);       // 步骤 1：读当前值
       mmio_write::<u16>(addr, cur | (1 << 1)); // 步骤 2：修改（OR 上 bit 1）
                                                  // 步骤 3：写回
   }
   ```

   **为什么不能直接写**？

   ```rust
   // ❌ 如果直接写：
   mmio_write::<u16>(addr, 1 << 1);  // 写入 0b0000_0000_0000_0010
   // 后果：CARD_INT（bit 8）原来是 1，被这一写变成了 0！
   // CARD_INT 从此再也不触发中断，WiFi 收不到数据了。
   ```

   所以必须先读当前值，修改目标位，再写回。这是 MMIO 编程的基本规则。

   ### 4.2 完整的轮询模式（旧代码）

   ```rust
   fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
       // Phase 1：自旋 1000 次，大约 50 微秒
       for _ in 0..1000 {
           let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);  // 读 STATUS
           if norm & (1 << 15) != 0 {  // ERROR 位被置位？
               // ...处理错误...
           }
           if norm & bit != 0 {  // 等的位被置位了？
               self.write::<u16>(SDHCI_INT_STATUS_NORM, bit);  // W1C 清除
               return Ok(());
           }
       }
       // Phase 2：没等到，进入慢速等待
       for i in 0..200_000 {
           let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
           if norm & bit != 0 {
               self.write::<u16>(SDHCI_INT_STATUS_NORM, bit);
               return Ok(());
           }
           yield_now();  // 让出 CPU ← 问题在这里
       }
       // 超时
   }
   ```

   ### 4.3 中断驱动的等待（新代码）

   ```rust
   fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
       // Phase 1 不变（快速自旋）

       // Phase 2：中断驱动等待
       for i in 0..20 {
           // 步骤 1：check before blocking
           if let Some(result) = self.check_int_status(bit) { return result; }

           // 步骤 2：仅 XFER_COMPLETE 启用中断信号
           if bit == NORM_INT_XFER_COMPLETE {
               irq::unmask_xfer_complete_signal();
               // → 写 SIG_EN：在原有值上 OR (1<<1)
               // → SIG_EN 现在有 bit1=1
               // → 下次硬件置 INT_STATUS[bit1]=1 时，中断会触发
           }

           // 步骤 3：阻塞自己
           let timed_out = crate::runtime::delay().block_timeout(10);
           // → WaitQueue::wait_timeout(10ms)
           // → 任务标记为 BLOCKED
           // → 10ms 超时，或 ISR 唤醒

           // 步骤 4：check after wake
           if let Some(result) = self.check_int_status(bit) { return result; }
       }
       // 超时：选择性清除
       self.write::<u16>(SDHCI_INT_STATUS_NORM, NORM_INT_ERROR | bit);
   }
   ```

   `check_int_status` 辅助函数：

   ```rust
   fn check_int_status(&self, bit: u16) -> Option<Result<(), SdioError>> {
       let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
       if norm & (1 << 15) != 0 {  // ERROR
           // 读错误详情、清除、复位 DAT 线、返回 Err
           return Some(Err(...));
       }
       if norm & bit != 0 {  // 等的位到了
           self.write::<u16>(SDHCI_INT_STATUS_NORM, bit);  // W1C 清除
           return Some(Ok(()));
       }
       None  // 继续等
   }
   ```

   ---

   ## 第 5 节：常见错误

   ### 错误 1：在 ISR 里 W1C 清除锁存位

   ```rust
   // ❌ 错误：ISR 里 W1C 清除了 XFER_COMPLETE
   if norm & NORM_INT_XFER_COMPLETE != 0 {
       mmio_write::<u16>(STATUS, NORM_INT_XFER_COMPLETE);  // W1C!
       wake_task();
   }

   // 之后任务醒来：
   let norm = read(STATUS);
   // XFER_COMPLETE 位 = 0 ← 已经被 ISR 消费了！
   // 任务永远看不到这个事件 → 必然超时
   ```

   **正确**：ISR 只 mask SIG_EN，不碰 STATUS。

   ### 错误 2：写 SIG_EN 时覆盖了 CARD_INT

   ```rust
   // ❌ 错误：直接写，覆盖了 CARD_INT 位
   mmio_write(SIG_EN, 1 << 1);  // 只保留 bit1，bit8(CARD_INT)丢掉了

   // ✅ 正确：读-改-写
   let cur = mmio_read(SIG_EN);
   mmio_write(SIG_EN, cur | (1 << 1));
   ```

   ### 错误 3：全量 W1C 清除 STATUS

   ```rust
   // ❌ 错误：超时时全量清除
   let sts = mmio_read(STATUS);   // 读到所有位，包括可能新来的 CARD_INT
   mmio_write(STATUS, sts);        // 全清了！CARD_INT 丢了！

   // ✅ 正确：只清需要清的位
   mmio_write(STATUS, NORM_INT_ERROR | bit);
   ```

   ### 错误 4：忘记 mask，ISR 重入

   ISR 中处理 XFER_COMPLETE 后，如果不 mask SIG_EN，FAULT 位还在 STATUS 里，ISR 返回后 CPU 立刻再次进入 ISR → 无限循环，系统卡死。这就是为什么 ISR 必须 mask。
