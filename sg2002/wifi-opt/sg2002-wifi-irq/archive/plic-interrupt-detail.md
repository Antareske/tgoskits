# PLIC 中断：从硬件信号到 Rust 函数

这份文档解释"硬件中断发生后，CPU 怎么一步步跳到我们的 `sdhci_irq_handler` 函数"。

---

## 1. 物理层：一根导线

SDHCI 控制器和 PLIC 中断控制器之间，有一根物理导线。

```
SDHCI 控制器                          PLIC 中断控制器
┌──────────────┐                    ┌──────────────────┐
│              │                    │                  │
│  IRQ 生成    │────── 导线 ──────→ │  中断源输入 #38  │
│  电路       │     高电平=有事     │                  │
│              │     低电平=没事     │                  │
└──────────────┘                    └────────┬─────────┘
                                             │
                                    PLIC 内部逻辑：
                                    ① 记录：源 #38 有中断待处理
                                    ② 查优先级
                                    ③ 选目标 CPU
                                    ④ 拉高连到 CPU 的那根线
                                             │
                                             ▼
                                    ┌──────────────────┐
                                    │  RISC-V CPU      │
                                    │  Hart 0          │
                                    │  (我们的 SG2002  │
                                    │   只有一个 hart) │
                                    └──────────────────┘
```

SDHCI 把这根线拉高（从 0V 变到 3.3V 或类似的电平变化），PLIC 检测到这个变化。

拉高的条件：`(INT_STATUS & SIG_EN) != 0`——只要任何一个被使能的 STATUS 位为 1，这根线就是高电平。

---

## 2. PLIC 层：从"源 #38 有中断"到"通知 CPU"

### 2.1 PLIC 是什么

PLIC（Platform-Level Interrupt Controller）是一个**中断路由器**。

打个比方：一栋办公楼（SoC）里有很多部门（硬件设备：SDHCI、UART、定时器……）。每个部门有一部内线电话（中断线），都接到前台（PLIC）。前台有一部外线电话直通老板（CPU）。

前台的工作：
1. 多个部门的内线同时响 → 按优先级决定先接哪个
2. 接起电话，问清楚是哪个部门、什么事
3. 用外线通知老板："SDHCI 部门有事找你"
4. 老板处理完 → 告诉前台"处理完了"→ 前台挂掉这通内线，可以接下一个

### 2.2 PLIC 的寄存器接口

PLIC 本身也是一组 MMIO 寄存器（在地址 `0x0C00_0000`）。我们通过 `ax_riscv_plic` crate 操作它。关键的三个寄存器：

| PLIC 寄存器 | 操作 | 含义 |
|:---|:---|:---|
| **ENABLE** | 写 | "源 #38 的中断我关心，请前台接听" |
| **CLAIM** | 读 | "老板问：哪个部门找我？"→ 返回源编号（如 38） |
| **COMPLETE** | 写 | "老板说：38 号的事处理完了，前台可以挂电话了" |

### 2.3 claim/complete 协议

这是 PLIC 的核心协议，理解它很重要：

```
                       PLIC                           CPU
                        │                              │
   SDHCI 拉高导线 ────→ │ 记录: 源#38 pending          │
                        │ 优先级够 → 拉高 CPU 中断线    │
                        │                              │
                        │ ─── 外部中断 #9 ───→          │ CPU 收到中断
                        │                              │ CPU 读 PLIC CLAIM 寄存器
                        │ ←── 读 CLAIM ─────────────── │
                        │ 返回: 38                      │ ← 知道是源 #38
                        │ 清除 pending，拉低 CPU 中断线 │
                        │                              │
                        │                              │ CPU 调用 sdhci_irq_handler()
                        │                              │ CPU 处理完毕
                        │                              │
                        │ ←── 写 COMPLETE(38) ──────── │
                        │ 源 #38 可以被再次触发          │
                        │                              │
```

**关键点**：

- **CLAIM 是一次性的**：读 CLAIM 寄存器会同时做两件事——返回源编号 + 清除 PLIC 内部的 pending 标志。同一个中断源在读 CLAIM 和写 COMPLETE 之间不会再次触发（即使 SDHCI 导线还是高电平）。
- **COMPLETE 是重新使能**：写 COMPLETE 告诉 PLIC "我处理完了，这个源可以再次触发中断了"。如果不写 COMPLETE，源 #38 永远不会再触发中断。
- **电平触发模型**：PLIC 是电平触发的。只要 SDHCI 的导线是高 AND 该源被使能，PLIC 就认为有中断。这也是为什么 ISR 必须 mask SIG_EN——不 mask 的话，就算你读了 CLAIM，只要 SIG_EN 还开着、STATUS 位还锁存着，导线就还是高，PLIC 马上又 pending，ISR 返回后 CPU 立刻又进来。

---

## 3. RISC-V CPU 层：从"收到中断"到"跳转到处理函数"

### 3.1 RISC-V 的中断类型

RISC-V 定义了三种"本地中断"，它们不是通过 PLIC 路由的，而是 CPU 内部直接产生的：

| 中断 | 编号 | 含义 | 谁产生 |
|:---|:---|:---|:---|
| Software | 1 | 核间中断（IPI） | 另一个 CPU 核心通过 SBI 发送 |
| Timer | 5 | 定时器中断 | 硬件定时器（mtimecmp） |
| External | 9 | 外部中断 | **PLIC 转发的**——这就是我们关心的 |

当 PLIC 决定通知 CPU 时，它拉高的是 CPU 的"外部中断"线（编号 9）。

### 3.2 中断使能：三道开关

RISC-V CPU 内部有三道开关，控制是否响应中断。

```
第一道：全局中断使能（sstatus.SIE 位）
  SIE=0 → 所有中断都不响应
  SIE=1 → 允许响应（但还要看下面两道）

第二道：每种中断的使能（sie 寄存器）
  sie[1] = SSIE → Software 中断
  sie[5] = STIE → Timer 中断
  sie[9] = SEIE → External 中断（PLIC 来的）

第三道：中断委托（sstatus.SPP 位 / medeleg 寄存器）
  SPP=1 → 中断在 Supervisor 模式处理（我们的情况）
  SPP=0 → 中断在 Machine 模式处理
```

三道的开关都在初始化时设置。在 `plic.rs` 的 `enable_local_interrupts()` 中：

```rust
fn enable_local_interrupts() {
    unsafe {
        sie::set_ssoft();   // sie[1] = 1  ← 允许 Software 中断
        sie::set_stimer();  // sie[5] = 1  ← 允许 Timer 中断
        sie::set_sext();    // sie[9] = 1  ← 允许 External 中断（PLIC）
    }
}
```

全局 SIE 在进入用户态之前由调度器打开。

### 3.3 CPU 收到中断后的硬件行为

当三道开关都打开，PLIC 拉高外部中断线时，CPU 硬件自动执行以下步骤（不需要软件参与）：

```
步骤 1：保存当前的 PC（程序计数器）到 sepc 寄存器
        （记住"打断前在哪一行代码"，等中断处理完要回去）

步骤 2：保存当前的 sstatus 到 sstatus.SPP 位
        （记住"打断前是什么特权模式"）

步骤 3：把 sstatus.SIE 清零
        （进入 ISR 期间禁止响应其他中断——中断嵌套关）

步骤 4：根据中断类型计算跳转地址
        中断类型 = 读 scause 寄存器判断是哪种中断
        如果是 External 中断 (cause=9)：
          跳转地址 = stvec（Supervisor Trap Vector）寄存器的值

步骤 5：PC = stvec
        开始执行中断处理函数
```

**stvec 是什么？**

stvec 是一个 CPU 寄存器，里面存了一个地址。这个地址在初始化时由 OS 写入——就是 OS 的中断入口函数。所有中断（Software、Timer、External）都先跳到这个地址。

在我们的系统中，stvec 指向 `somehal` 的 trap 入口（一段汇编），它做的事情：

```asm
trap_entry:
    # 保存所有通用寄存器到栈上（x1~x31）
    # 把 scause（中断原因）和 sepc（返回地址）作为参数
    # 调用 Rust 函数 handle_trap(scause, sepc, ...)
```

### 3.4 handle_trap → begin_irq → claim → handler

Rust 层面的 `handle_trap` 检查 `scause`：

```
scause 的最高位 = 1 → 这是中断（不是异常）
scause 的低位   = 9 → 外部中断（来自 PLIC）

→ 调用 begin_irq(raw=9)
```

`begin_irq(9)` 在 `plic.rs` 中：

```rust
pub fn begin_irq(raw: usize) -> Option<ActiveIrq> {
    match classify_riscv_trap(raw) {
        RiscvTrapIrq::External => begin_external_irq(),
        // ...
    }
}

fn begin_external_irq() -> Option<ActiveIrq> {
    let source = claim_external_irq_source()?;  // 读 PLIC CLAIM → 得到 38
    Some(ActiveIrq {
        irq: (source.get() as usize).into(),     // irq = 38
        completion: Completion::Plic(source),     // 记住：回去后要写 COMPLETE(38)
    })
}

fn claim_external_irq_source() -> Option<NonZeroU32> {
    let handler = get_irq_handler()?;
    handler.claim_current()  // 读 PLIC 的 CLAIM 寄存器 → 返回 38
}
```

`begin_irq` 返回 `ActiveIrq { irq: 38, completion: Plic(38) }` 之后，irq 框架拿到 `irq=38`，找到之前注册的处理函数 `sdio1_irq_handler`，调用它。

### 3.5 处理完后的清理

处理函数返回后，`ActiveIrq` 被 drop：

```rust
impl Drop for ActiveIrq {
    fn drop(&mut self) {
        if let Completion::Plic(source) = self.completion {
            complete_external_irq_source(source);
            // → 写 PLIC COMPLETE(38)
            // → 告诉 PLIC："源 #38 处理完了，可以接下一个了"
        }
    }
}
```

然后 trap 入口恢复之前保存的寄存器，执行 `sret` 指令（Supervisor Return），CPU 硬件：
1. 恢复 sstatus.SIE（重新允许中断）
2. PC = sepc（跳回打断前的代码）
3. 被打断的代码继续执行，仿佛什么都没发生

---

## 4. 注册链：handler 是怎么和 IRQ#38 关联的

前面解释了"中断发生后怎么走到 handler"。这里解释"handler 是怎么被注册的"。

### 4.1 设备树

硬件连接信息存在设备树（FDT，Flattened Device Tree）里。这是一段二进制数据，由固件（U-Boot）在启动时传给内核。它描述硬件的连接关系。

SDIO1 控制器的节点大概是这样的：

```
sdio1@4320000 {
    compatible = "cvitek,cv1800b-sdhci";
    reg = <0x04320000 0x1000>;          // MMIO 基地址 + 大小
    interrupts-extended = <&plic 38>;    // 连到 PLIC 的 38 号源
};
```

### 4.2 驱动注册

在 `drivers/ax-driver/src/net/aic8800.rs` 的 probe 函数中：

```rust
fn probe(probe: ProbeFdt<'_>) -> Result<(), OnProbeError> {
    // ...

    // 步骤 1：从设备树解析 IRQ 编号（返回 38）
    let irq = resolve_fdt_irq(&info)?;
    // irq = 38

    // 步骤 2：创建 SDHCI 控制器实例，初始化硬件
    let host = CviSdhci::new(sdio1_paddr);
    host.enable_interrupts_irq()?;
    // 这一步内部：
    //   设置 STS_EN = CMD_COMPLETE | XFER_COMPLETE | BUF_WR_READY | BUF_RD_READY | CARD_INT
    //   设置 ERR_STS_EN = 全部错误位
    //   设置 SIG_EN = CARD_INT （只有它；XFER_COMPLETE 初始为 0）
    //   → STATUS 和 STS_EN 全开，SIG_EN 只开 CARD_INT

    // 步骤 3：注册 ISR
    let irq_handle = axklib::irq::request_shared_disabled(irq, sdio1_irq_handler)?;
    // 这一步内部：
    //   调用 PLIC 的 enable_source(38)
    //     → 设置源 #38 的优先级 = 1
    //     → 把源 #38 在 PLIC 的 ENABLE 寄存器中对应位置 1
    //     → 把 handler 存入 irq_domain 的查找表
    //   返回 handle（用于后续 enable/disable）
}
```

`request_shared_disabled` 做三件事：
1. 在 PLIC 中使能源 #38（设优先级 + 写 ENABLE 位）
2. 把 `sdio1_irq_handler` 函数指针存入一个全局表，key=38
3. 返回一个 handle（此时中断是 disabled 状态，需要后续 enable）

### 4.3 最终注册链全景

```
设备树 (FDT)                          PLIC 寄存器
"sdio1 连 PLIC 源 38"                 ENABLE[38] = 1
         │                                   │
         ▼                                   ▼
resolve_fdt_irq()               PLIC 内部: 源 38 可以路由到 CPU
返回 irq_id = 38
         │
         ▼
request_shared_disabled(38, handler)
         │
         ├──→ plic.enable_source(38)
         │       ├── 设优先级 = 1
         │       └── ENABLE[38] = 1    ← PLIC 硬件层面使能
         │
         └──→ irq_domain.register(38, sdio1_irq_handler)
                 └── 存入查找表: {38 → sdio1_irq_handler}
```

---

## 5. 完整时序

把上面全串起来，一次 CARD_INT 中断的完整路径：

```
时刻 0：WiFi 芯片有数据 → SDIO CARD_INT 信号
         → SDHCI: INT_STATUS[bit8] = 1
         → SDHCI: (STATUS & SIG_EN) != 0 → IRQ 线拉高

时刻 1：PLIC 检测到源 #38 的导线变高
         → PLIC 内部置 pending=1
         → 优先级 1 ≥ 阈值 → 拉高连到 CPU 的外部中断线

时刻 2：CPU 检测到外部中断
         条件: sstatus.SIE=1, sie.SEIE=1
         → 硬件自动保存 PC→sepc, 清 SIE, 跳转到 stvec

时刻 3：trap_entry (汇编)
         → 保存 x1~x31 寄存器到栈
         → 调 handle_trap(scause=9)

时刻 4：handle_trap → begin_irq(9)
         → begin_external_irq()
         → claim_current()
         → 读 PLIC CLAIM 寄存器
         → PLIC 返回 38，同时清 pending，拉低 CPU 中断线
         → ActiveIrq { irq=38, completion=Plic(38) }

时刻 5：irq 框架用 irq=38 查表
         → 找到 sdio1_irq_handler
         → 调用 sdio1_irq_handler()

时刻 6：sdio1_irq_handler (ax-driver 的 trampoline)
         → sdhci_cv1800::irq::sdhci_irq_handler(0)
         → 读 INT_STATUS_NORM
         → 看到 CARD_INT=1, XFER_COMPLETE=0
         → mask CARD_INT 信号
         → 调用 card_irq_callback = sdio1_irq_handler (aic8800 的)

时刻 7：aic8800::sdio1_irq_handler (bus.rs)
         → bus.rx.irq_pending = true
         → bus.rx.irq_waker.wake()
         → 返回到 ISR

时刻 8：ISR 返回 → trap 出口
         → ActiveIrq 被 drop
         → complete_external_irq_source(38)
         → 写 PLIC COMPLETE(38)
         → 恢复寄存器，sret
         → 回到被打断的代码

时刻 9：调度器看到 RX 线程被唤醒
         → 调度 RX 线程
         → RX 线程读 FIFO，处理数据
         → 最后 unmask CARD_INT 信号（重新允许 CARD_INT 触发）
```

---

## 6. 为什么 unmask 必须在任务上下文而不是 ISR 里

ISR 负责最快地响应硬件并做最少的事。CARD_INT 的处理模式是：

- **ISR 里**：mask CARD_INT + 设置 pending flag + 唤醒线程（总耗时几微秒）
- **线程里**：读数据 + 处理 + 分发（可能耗时几毫秒）

如果在 ISR 里处理完整的数据读取，ISR 会阻塞很长时间，期间其他中断无法响应。如果在 ISR 里 unmask CARD_INT，而芯片还有数据待读（STATUS 的 CARD_INT 还在锁存），ISR 返回后会立刻再次进入 ISR → 死循环。

所以 unmask 必须在线程处理完数据、芯片不再有数据之后才做。

---

## 7. 总结

```
SDHCI 导线拉高
  → PLIC 源 #38 pending
  → PLIC 拉高 CPU 外部中断线
  → CPU: 硬件保存 PC→sepc, 清 SIE, 跳转 stvec
  → trap 汇编: 保存寄存器, 调 handle_trap
  → begin_external_irq: 读 PLIC CLAIM, 得到 38
  → irq 框架查表 #38 → sdio1_irq_handler
  → sdhci_irq_handler: 读 SDHCI 寄存器, mask SIG_EN, 调 callback
  → callback: 设置 flag, 唤醒线程
  → 返回 → 写 PLIC COMPLETE(38) → sret → 回到原代码
  → 线程被调度: 读数据, unmask SIG_EN
```

从 SDHCI 导线变高到 `sdhci_irq_handler` 被调用，中间经过了 PLIC（路由 + 优先级）、CPU trap 机制（保存上下文 + 跳转）、汇编入口（寄存器保存）、Rust 中间层（claim + 查表）。每一步都是硬件或 OS 基础设施自动完成的，驱动层只需要关心"我的 handler 函数里做什么"。
