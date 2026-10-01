# 为什么多看了一眼时间，WiFi 上传就快了 10 倍？

> 从 CPU 指令执行、MMIO 到 fence，逐步解释本次发现的根因。

## 0. 现象

两份内核，代码只差一处：诊断计数器在 `poll_int_status` 入口多调了一次 `now_nanos()`。结果 TX 上传从 **1.16 Mbps 变成 11.0 Mbps**，差 9.5 倍。

---

## 1. 基础知识

### 1.1 CPU 如何执行指令

程序是一条条指令组成的。编译器把代码翻译成机器指令后，CPU 逐条执行。以 RISC-V 为例：

```asm
lw  a0, 0(t0)      # 从地址 t0 加载 32 位到寄存器 a0
add a1, a0, a2     # a1 = a0 + a2
sw  a1, 0(t3)      # 把 a1 存到地址 t3
```

程序员的理解是：这三条指令按顺序执行，先 lw，再 add，最后 sw。

### 1.2 乱序执行

但当代 CPU 实际执行时不是这样的。CPU 内部有多个执行单元（多个 ALU、多个 load/store 单元等），它会在指令流里找到"数据没依赖"的指令提前执行。

例如这三条：

```asm
lw  a0, [addr1]     # ① 加载 addr1
lw  a1, [addr2]     # ② 加载 addr2 —— 和 ① 没有数据依赖
add a2, a0, a1      # ③ a2 = a0 + a1 —— 依赖 ① 和 ② 的结果
```

指令 ② 和 ① 访问不同地址、无数据依赖。CPU 可以把 ① 和 ② 同时发给两个 load 单元执行。③ 必须等 ① 和 ② 都完成才能执行。

这种并行只影响延迟，不影响结果——对程序员透明。

### 1.3 Store Buffer

涉及写操作时情况更复杂。CPU 通常不直接写回内存——因为内存访问很慢（几百个周期）。它先把数据写到一个叫 **store buffer** 的内部小缓冲区里：

```
sw  a0, [addr1]    # 不是立刻写到内存，而是放进 store buffer
lw  a1, [addr2]    # 读操作可能比上面的写更早到达内存
```

store buffer 是 FIFO 队列，CPU 不停往里面塞写操作，总线控制器在后台逐个排空。**写操作和读操作在时间线上可能互相穿插。** 但 CPU 保证：同一个 CPU 从同一个地址读，会先检查自己的 store buffer，如果有待写的值就直接返回那个值。这叫 store-to-load forwarding，让你感觉"写完了才读"。

### 1.4 MMIO 和普通内存的区别

**普通内存**：地址指向 RAM。数据没有副作用——读一次和读一百次是一样的（除非有其他 CPU 改变了它）。

**MMIO（Memory-Mapped I/O）**：地址指向硬件设备的寄存器。读这个地址有副作用：
- 读硬件状态寄存器（如 SDHCI 的 `INT_STATUS_NORM`），返回的是硬件当前的实时状态
- 每次读都必须在硬件总线上真实发生——不能从 CPU 缓存里拿
- 连续两次读同一个 MMIO 地址，结果可能不同（因为硬件状态变了）

这就是为什么 MMIO 访问必须用 `volatile` 语义——编译器层面保证每次读写都真实发生，不能被优化掉。

但 `volatile` 只管编译器，管不了 CPU 硬件。

### 1.5 Fence 指令

RISC-V 的 `fence` 指令格式：

```
fence <前一组>, <后一组>
```

其中每组可以是 `i`（input/load）、`o`（output/store）、`r`（read）、`w`（write）的组合。

`fence iorw, iorw` 的意思是：**这条指令之前的所有内存读写（无论方向），必须在之后的所有内存读写开始之前，在硬件层面完成。**

具体来说，用了 fence 之后，CPU 会：
1. 等待 store buffer 里所有待写的数据被总线接收
2. 等待所有正在进行的 load 操作收到数据
3. 然后才开始执行 fence 之后的 load/store 指令

不涉及 MMIO 的普通程序几乎不需要 fence——因为 store-to-load forwarding 保证了单核上"看起来顺序执行"。但 MMIO 不一样——**硬件状态的变化不由本 CPU 控制**。另一个设备（SDHCI 控制器、WiFi 模组）改变了状态，CPU 必须读到真实的新值，不能依赖 forwarding。

---

## 2. 本场景中发生了什么

### 2.1 当前代码的等待循环

`poll_int_status` 的 Phase 1 是 1000 次自旋：

```rust
for _ in 0..1000 {
    let status = mmio_read(INT_STATUS_NORM);  // read_volatile
    if status & bit != 0 { return; }
    core::hint::spin_loop();                  // pause / nop
}
```

`mmio_read` 是 `read_volatile`——编译器保证每次都实际发出读指令。

`core::hint::spin_loop()` 在 RISC-V 上生成 `pause` 指令，效果等于 `nop`（空操作）。

### 2.2 问题出在哪

RISC-V 规范中，`volatile` 只约束编译器，CPU 硬件不认得"这个地址是 MMIO"这件事（除非通过 PMA 属性标记，但 SG2002 的 bare-metal 环境通常不设）。CPU 看到的就是两条连续的 load 指令，它可以：

1. 把连续两条 load 同时发出（多发射）
2. 在 load 之间穿插其他不相关的指令执行
3. 更关键的是：**前面的 MMIO 写操作还在 store buffer 里没提交时，后面的 MMIO 读就已经发出去了**

第三点是最可能的根因。一次 TX 传输的流程是：

```
① mmio_write(SDHCI_BUFFER, data)  ← 128 次写往 PIO 数据端口
② 然后立刻进入 poll_int_status，频繁 mmio_read(INT_STATUS_NORM)
```

① 的最后几个 `mmio_write` 可能还在 store buffer 里排队。此时 ② 已经开始读 `INT_STATUS_NORM` 了。CPU 的 store-to-load forwarding 机制只对**同一 CPU 写到同一地址**的情况有效——但 `SDHCI_BUFFER`（0x20）和 `INT_STATUS_NORM`（0x30）是不同的地址。CPU 不知道它们属于同一个设备，也不知道 0x30 的值依赖于 0x20 的写入完成。

结果：读 `INT_STATUS_NORM` 拿到的可能是设备端还没消化完 buffer 数据时的旧状态。`BUF_WR_READY` 还没置位——不是因为硬件没准备好，而是因为**CPU 还没确认 buffer 写入已经提交到设备**。

### 2.3 `now_nanos()` 如何意外修了这个问题

诊断计数器在 Phase 1 之前加了一行：

```rust
let t_entry = delay().now_nanos();  // → ax_hal::time::monotonic_time_nanos()
```

RISC-V 上 `monotonic_time_nanos()` 读取硬件定时器。SG2002 的硬件定时器（`mtime`）映射在 MMIO 地址空间。这是一次 `read_volatile` 到一个与 SDHCI 完全不同的地址。

这次读取：
1. 走了完整的硬件总线往返
2. 因为读取的是不同地址域的 MMIO 寄存器，总线控制器从 SDHCI 域切换到定时器域再切回来，自然产生一个同步点
3. 在 `now_nanos()` 返回之后，store buffer 中的 SDHCI 写入已经提交到设备

之后 Phase 1 的第一次 `mmio_read(INT_STATUS_NORM)` 读到的就是设备真实状态了。

### 2.4 `spin_loop()` 为什么是问题的一部分

`core::hint::spin_loop()` 的目的是提示 CPU"我在忙等，降低功耗"。RISC-V 上它编译成 `pause` 指令，效果等同 `nop`。

`nop` 在密集的 MMIO 轮询中起了反作用——它给 CPU 的指令调度器留了一个"缝"。CPU 看到连续的 `load + nop + load + nop` 模式时，可能会在 nop 的间隙里插入其他不相关的操作（如 store buffer flush 的后台处理），这进一步稀释了轮询的有效时间窗口。

**去掉 `spin_loop()`，1000 次连续的 `mmio_read` 会形成一个依赖链：** 每次读取的结果决定是否跳出循环，CPU 必须等这次读完成才能判断分支。这种控制依赖让 CPU 无法提前发出后面的读，但也保证了每次读之间没有插入无关操作。

### 2.5 为什么 yield_now 方案不慢

Part 1 的 `yield_now()` 自旋循环是：

```rust
for _ in 0..200_000 {
    if mmio_read(INT_STATUS_NORM) & bit != 0 { return; }
    yield_now();  // 让出 CPU
}
```

`yield_now()` 内部的任务切换需要：
1. 保存当前任务的寄存器快照到内存（多次 store）
2. 原子操作调度器 run_queue
3. 切换特权级（`ecall` / `mret`）

第 3 步的关键：RISC-V 的 `ecall` 和 `mret` 指令**自带隐式 fence 效果**——它们在规范中保证之前的所有内存操作都已完成。所以每次 yield 回来之后，CPU 的 store buffer 是空的，下一次 MMIO 读一定是真实的。

中断驱动的 XFER_COMPLETE 方案用 `delay_ms(10)` + ISR 替换了 `yield_now()`，但也丢失了这个隐式 fence。

---

## 3. 修复方案

三个方案，按开销递增：

### 方案 A：去掉 `spin_loop()`（最简单）

```rust
for _ in 0..PHASE1_SPIN_ITERS {
    if let Some(result) = self.poll_status_once(bit) { return result; }
    // 删除 core::hint::spin_loop();
}
```

理由：连续的 MMIO 读自带控制依赖（读结果立刻用于分支判断），CPU 无法提前发出下一次读。1000 次连续 `mmio_read` 形成一条紧耦合的读链。需要验证连续 MMIO 读是否会导致 SDIO 总线过载或影响其他设备。

### 方案 B：Phase 1 入口 fence 一次（对标 now_nanos() 效果）

```rust
// Phase 1 入口处
core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
for _ in 0..PHASE1_SPIN_ITERS {
    if let Some(result) = self.poll_status_once(bit) { return result; }
    core::hint::spin_loop();
}
```

`fence(SeqCst)` 在 RISC-V 上生成 `fence rw, rw`，清空 store buffer 并等待所有待处理的 load 完成。RISC-V 规范还保证了**同一地址的 MMIO 读按程序顺序执行**——1000 次读同一个 `INT_STATUS_NORM` 不会互相重排。所以入口处 fence 一次应该够用。这是最接近 `now_nanos()` 行为的方案。

### 方案 C：每轮迭代 fence

```rust
for _ in 0..PHASE1_SPIN_ITERS {
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    if let Some(result) = self.poll_status_once(bit) { return result; }
    core::hint::spin_loop();
}
```

每次迭代多一条 `fence rw, rw` 指令（~几十个周期），1000 次累积额外开销约几万周期，仍在微秒级别。最保险但略贵。

---

## 4. 关键结论

1. **`volatile` 只管编译器，管不了 CPU 硬件的乱序。** 对 MMIO 的正确性保障需要 fence。
2. **Phase 1 的 1000 次自旋本身是够用的**——加了 fence 后，TX 从 1.16 Mbps 跳到 11.0 Mbps，说明硬件响应时间确实在 50µs 窗口内。
3. **之前的 0.85 Mbps 不是因为 10ms 睡眠的架构问题**——是因为 Phase 1 的轮询在缺少 fence 时大量漏读，导致频繁掉入 Phase 2。10ms 是受害者，不是凶手。
4. **中断驱动方案（XFER_COMPLETE ISR）本身是正确的**——它消除了 50ms 调度惩罚。但它附带消除了 `yield_now()` 中隐含的 fence 效果，这是未被察觉的退步。
