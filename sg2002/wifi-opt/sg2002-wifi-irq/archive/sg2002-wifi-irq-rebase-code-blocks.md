# sg2002/wifi-irq 分支工作：与最新 dev 冲突处的整块代码对照

- 分支：`sg2002/wifi-irq`
- 变更前基线（分叉点）：`e04580c5c`（2026-08-06，fix(starry-cred) #1906）
- 变基目标（主线最新 dev）：`220af445b`
- 变基后分支尖端：`5b67b0ab0`

## 冲突面界定

自分叉点以来**双方都改动过**的文件（即冲突面）只有本目录列出的 5 个：分支侧是内容变更，dev 侧的唯一改动是主线 #1951 的纯路径迁移（对每个文件均为 0 行内容变化）。因此：
- 变基全程**零内容冲突**：git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对确认内容一致；
- 下文对每个文件列出该冲突位置的全部整块，"变更前"为分叉基线（dev 未动其内容时的版本）上的整块源码，"变更后"为变基后当前分支的整块源码；整块按名字配对并做全文比对，内容一致的整块不展示。

**不展示的内容**（按要求排除）：
- `os/arceos/modules/axruntime/src/wifi_glue.rs`、`os/StarryOS/configs/board/licheerv-nano-sg2002.its`：分支改动，但 dev 自分叉点以来零提交触碰这两个文件，rebase 无冲突；
- `licheerv-nano-sg2002.toml`：分支两笔改动（加回 `aic8800-wifi` feature 后移除）净差为零，dev 也未触碰；
- dev 自分叉点以来的新增内容（如 #2106 dma coherency、#1956 锁统一）未触及分支改动文件，不影响本分叉。

分支提交（重放后）：

- `0581f495c fix&test(sg2002,wifi): fix wifi boot & include sg2002 wifi config`
- `df750c0db fix(sdhci-cv1800): add interrupt-driven PIO transfer completion`
- `dba592c18 feat(sdhci-cv1800): add per-bit poll_int_status diagnostic counters`
- `a6d99305e Revert "feat(sdhci-cv1800): add per-bit poll_int_status diagnostic counters"`
- `98e90cd3b fix(sdhci-cv1800): drain store buffer before Phase 1 MMIO polling`
- `b72581fba fix(sdhci-cv1800): use selective W1C to preserve XFER_COMPLETE across command and error paths`
- `5b932ea32 fix(sdhci-cv1800): consume XFER_COMPLETE in error and timeout exit paths`
- `6c983388e chore(sdhci-cv1800): translate newly-introduced comments to Chinese for consistency`
- `6e4bbdd63 chore(sg2002): remove aic8800-wifi feature from licheerv-nano-sg2002 base config`
- `5b67b0ab0 fix(sdhci-cv1800): close lost-wakeup window in Phase 2 XFER_COMPLETE wait`

---

## 目录

- drivers/blk/sdhci-cv1800/src/irq.rs
- drivers/blk/sdhci-cv1800/src/lib.rs
- drivers/blk/sdhci-cv1800/src/regs.rs
- drivers/blk/sdhci-cv1800/src/runtime.rs
- drivers/net/aic8800/src/fdrv/thread/rx.rs

---

## 文件：`drivers/blk/sdhci-cv1800/src/irq.rs`

- 冲突面：路径冲突。dev 侧（自分叉点以来）对该文件的唯一改动是主线 #1951 的纯路径迁移（0 行内容变化）；分支侧的改动为内容变更。变基中 git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对内容一致。
- 变更前路径：`components/sdhci-cv1800/src/irq.rs` @ e04580c5c
- 变更后路径：`drivers/blk/sdhci-cv1800/src/irq.rs` @ HEAD

> 说明："变更前"为该位置在分叉基线（`e04580c5c`，即 dev 未动其内容时的版本）上的整块源码；"变更后"为变基后当前分支的整块源码。

整块数量：修改 8 / 新增 5 / 删除 0

#### `//! 文件头注释`（修改）
变更前（第 1–6 行）：
```rust
//! SDHCI 中断处理模块
//!
//! 设计模式：
//!   - ISR: 裸函数，只处理 CARD_INT (mask 信号 + 调用回调)
//!   - PIO: CviSdhci 的 wait_* 方法直接轮询 INT_STATUS 寄存器
//!   - 分离 ISR 和 PIO 事件避免竞态条件
```

变更后（第 1–22 行）：
```rust
//! SDHCI 中断处理模块
//!
//! 设计模式：
//!   - ISR: 处理 CARD_INT 和 XFER_COMPLETE 中断
//!     - CARD_INT: mask 信号 + 调用回调（通知 WiFi 驱动有数据可读）
//!     - XFER_COMPLETE: mask 信号 + 调用 PIO 唤醒回调（唤醒阻塞的任务）
//!   - PIO: CviSdhci 的 wait_* 方法在 Phase 1 直接轮询 INT_STATUS 寄存器，
//!     Phase 2 通过中断驱动等待（block_timeout_until）
//!
//! # Single-core assumption
//!
//! SIG_EN 的 RMW（task 侧 unmask 与 ISR 侧 mask）在单 hart 上仍可能被中断
//! 抢占：ISR 可能在 task 的 `mmio_read` 和 `mmio_write` 之间触发，task 的
//! 过期值可能重写 SIG_EN。此竞态不会丢事件：XFER_COMPLETE sticky 位由等待
//! 任务独占消费（ISR 从不 W1C），且 `block_timeout_until` 的条件检查与任务
//! 入队在同一关中断临界区内衔接——ISR 无论发生在锁内条件检查前（notify
//! 落空，sticky 位被条件观察到）、检查后入队前（不可能，同一关中断临界区）
//! 还是入队后（notify 命中队列），事件都不会错过。SIG_EN 被过期值重写
//! 最多产生一次多余 ISR（读状态无 XFER 位则空转）。SMP 平台下跨 hart 的
//! ISR 可在检查与入队之间触发（notify 落空、退化为有界 10ms 超时），需
//! 额外围栏和 per-hart INT_STATUS 分离；wifi_glue 的单核 debug_assert
//! 在 release 构建中被编译掉，不构成运行时防护。
```

#### `struct CallbackSlot {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 28–34 行）：
```rust
/// 回调槽：存储 ISR 调用的函数指针。
///
/// 使用 `AtomicUsize` 因为 `fn()` 在所有支持的平台上是指针宽度
/// （riscv64、aarch64、x86_64）。零值表示未注册，ISR 对此有防护。
struct CallbackSlot {
    ptr: AtomicUsize,
}
```

#### `impl CallbackSlot {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 36–66 行）：
```rust
impl CallbackSlot {
    const fn new() -> Self {
        Self {
            ptr: AtomicUsize::new(0),
        }
    }

    /// 注册回调函数。在初始化期间、IRQ 使能前调用一次。
    fn register(&self, cb: fn()) {
        self.ptr.store(cb as usize, Ordering::Release);
    }

    /// 调用已注册的回调（如果有）。
    ///
    /// # 安全性
    ///
    /// - 存储的值必须是先前通过 `register` 存储的有效 `fn()`。
    ///   零值（空槽）被防护，不会触发回调调用。
    /// - 单核假设：回调调用期间没有其他 hart 并发写入此槽。
    /// - 回调在硬中断上下文中运行，禁止：分配堆、持锁、调度、
    ///   调用同步写 UART 的 `log` 宏。
    unsafe fn invoke(&self) {
        let v = self.ptr.load(Ordering::Acquire);
        if v != 0 {
            // SAFETY: v 由 register() 以 `cb as usize` 存入。
            // fn() 和 usize 在所有支持的平台上大小相同。
            let cb: fn() = unsafe { core::mem::transmute::<usize, fn()>(v) };
            cb();
        }
    }
}
```

#### `struct SdhciIrqState {`（修改）
变更前（第 12–18 行）：
```rust
/// SDHCI 中断全局状态
struct SdhciIrqState {
    /// SDHCI MMIO 基地址（ISR 裸写用）
    base: AtomicUsize,
    /// CARD_INT 回调（通知上层驱动有数据可读）
    card_irq_callback: AtomicUsize, // fn() 的裸指针
}
```

变更后（第 68–76 行）：
```rust
/// SDHCI 中断全局状态
struct SdhciIrqState {
    /// SDHCI MMIO 基地址（ISR 裸写用）
    base: AtomicUsize,
    /// CARD_INT 回调（通知上层驱动有数据可读）
    card_irq_callback: CallbackSlot,
    /// XFER_COMPLETE PIO 唤醒回调（唤醒阻塞在 block_timeout_until 的任务）
    pio_wake_callback: CallbackSlot,
}
```

#### `impl SdhciIrqState {`（修改）
变更前（第 20–27 行）：
```rust
impl SdhciIrqState {
    const fn new() -> Self {
        Self {
            base: AtomicUsize::new(0),
            card_irq_callback: AtomicUsize::new(0),
        }
    }
}
```

变更后（第 78–86 行）：
```rust
impl SdhciIrqState {
    const fn new() -> Self {
        Self {
            base: AtomicUsize::new(0),
            card_irq_callback: CallbackSlot::new(),
            pio_wake_callback: CallbackSlot::new(),
        }
    }
}
```

#### `pub fn register_card_irq_callback(cb: fn()) {`（修改）
变更前（第 40–48 行）：
```rust
/// 注册 CARD_INT 回调函数
///
/// WiFi 驱动初始化时调用，注册一个函数用于在 ISR 中通知"卡有数据可读"。
/// 回调在硬中断上下文执行，禁止：持锁、分配堆、调度、调用 log。
pub fn register_card_irq_callback(cb: fn()) {
    SDHCI_IRQ_STATE
        .card_irq_callback
        .store(cb as usize, Ordering::Release);
}
```

变更后（第 99–105 行）：
```rust
/// 注册 CARD_INT 回调函数
///
/// WiFi 驱动初始化时调用，注册一个函数用于在 ISR 中通知"卡有数据可读"。
/// 回调在硬中断上下文执行，禁止：持锁、分配堆、调度、调用 log。
pub fn register_card_irq_callback(cb: fn()) {
    SDHCI_IRQ_STATE.card_irq_callback.register(cb);
}
```

#### `pub fn register_pio_wake_callback(cb: fn()) {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 107–114 行）：
```rust
/// 注册 PIO 唤醒回调函数
///
/// OS 胶水层在初始化时调用，注册一个函数用于在 ISR 中唤醒阻塞在
/// `block_timeout_until` 上的任务。回调在硬中断上下文执行，禁止：持锁、分配堆、
/// 调度、调用 log。
pub fn register_pio_wake_callback(cb: fn()) {
    SDHCI_IRQ_STATE.pio_wake_callback.register(cb);
}
```

#### `pub fn enable_irq_signals() {`（修改）
变更前（第 50–55 行）：
```rust
/// 使能 CARD_INT 中断信号（ISR 仅处理 CARD_INT，PIO 事件由轮询处理）
pub fn enable_irq_signals() {
    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    mmio_write::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize, NORM_INT_SIG_MASK);
    mmio_write::<u16>(base + SDHCI_ERR_INT_SIG_EN as usize, ERR_INT_SIG_MASK);
}
```

变更后（第 116–124 行）：
```rust
/// 使能 CARD_INT 中断信号（XFER_COMPLETE 由 poll_int_status 动态 un-mask）
pub fn enable_irq_signals() {
    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    mmio_write::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize, NORM_INT_SIG_MASK);
    mmio_write::<u16>(base + SDHCI_ERR_INT_SIG_EN as usize, ERR_INT_SIG_MASK);
}
```

#### `pub fn disable_irq_signals() {`（修改）
变更前（第 57–62 行）：
```rust
/// 禁用所有 SDHCI 中断信号
pub fn disable_irq_signals() {
    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    mmio_write::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize, 0);
    mmio_write::<u16>(base + SDHCI_ERR_INT_SIG_EN as usize, 0);
}
```

变更后（第 126–134 行）：
```rust
/// 禁用所有 SDHCI 中断信号
pub fn disable_irq_signals() {
    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    mmio_write::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize, 0);
    mmio_write::<u16>(base + SDHCI_ERR_INT_SIG_EN as usize, 0);
}
```

#### `fn rmw_norm_sig_en(base: usize, set: u16, clear: u16) {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 142–150 行）：
```rust
/// 对 `SDHCI_NORM_INT_SIG_EN` 执行 Read-Modify-Write。
///
/// RMW 本身不是原子的（可能被 ISR 抢占），但设计依赖 XFER_COMPLETE sticky bit
/// 自愈：即使本次写入携带过期值，中断线在 task 阻塞后重新断言，ISR 重新触发。
fn rmw_norm_sig_en(base: usize, set: u16, clear: u16) {
    let addr = base + SDHCI_NORM_INT_SIG_EN as usize;
    let cur = mmio_read::<u16>(addr);
    mmio_write::<u16>(addr, (cur & !clear) | set);
}
```

#### `pub fn unmask_xfer_complete_signal() {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 152–162 行）：
```rust
/// 启用 XFER_COMPLETE 中断信号（在 poll_int_status 阻塞前调用）
///
/// 注意：SDHCI ISR 收到 XFER_COMPLETE 后会立即 mask 掉该信号，
/// 因此每次阻塞前都需要重新调用此函数。
pub fn unmask_xfer_complete_signal() {
    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    rmw_norm_sig_en(base, NORM_INT_XFER_COMPLETE, 0);
}
```

#### `pub(crate) fn mask_card_irq_raw(base: usize, mask: bool) {`（修改）
变更前（第 64–72 行）：
```rust
/// 屏蔽/恢复 CARD_INT 信号（裸地址操作，ISR 安全）
pub(crate) fn mask_card_irq_raw(base: usize, mask: bool) {
    let addr = base + SDHCI_NORM_INT_SIG_EN as usize;
    let cur = mmio_read::<u16>(addr);
    mmio_write::<u16>(
        addr,
        (cur & !NORM_INT_CARD_INT) | (!mask as u16 * NORM_INT_CARD_INT),
    );
}
```

变更后（第 164–171 行）：
```rust
/// 屏蔽/恢复 CARD_INT 信号（裸地址操作，ISR 安全）
pub(crate) fn mask_card_irq_raw(base: usize, mask: bool) {
    if mask {
        rmw_norm_sig_en(base, 0, NORM_INT_CARD_INT);
    } else {
        rmw_norm_sig_en(base, NORM_INT_CARD_INT, 0);
    }
}
```

#### `pub fn sdhci_irq_handler(_irq: usize) {`（修改）
变更前（第 74–102 行）：
```rust
/// SDHCI 中断处理函数（注册到 PLIC）
///
/// 只处理 CARD_INT：mask 信号 + 调用回调。
/// PIO 事件（CMD_COMPLETE / BUF_RD_READY / XFER_COMPLETE）由 wait 函数直接轮询。
pub fn sdhci_irq_handler(_irq: usize) {
    SDHCI_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);

    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    if base == 0 {
        return;
    }

    let status = mmio_read::<u32>(base + SDHCI_INT_STATUS_NORM as usize);
    if status == 0 {
        return;
    }

    let norm = status as u16;
    SDHCI_LAST_NORM.store(norm, Ordering::Relaxed);

    if norm & NORM_INT_CARD_INT != 0 {
        SDHCI_CARD_INT_COUNT.fetch_add(1, Ordering::Relaxed);
        mask_card_irq_raw(base, true);
        let cb = SDHCI_IRQ_STATE.card_irq_callback.load(Ordering::Acquire);
        if cb != 0 {
            unsafe { core::mem::transmute::<usize, fn()>(cb)() };
        }
    }
}
```

变更后（第 173–218 行）：
```rust
/// SDHCI 中断处理函数（注册到 PLIC）
///
/// 处理两种中断：
/// - CARD_INT: mask 信号 + 调用回调（通知 WiFi 驱动有数据可读）
/// - XFER_COMPLETE: mask 信号 + 调用 PIO 唤醒回调（不消费锁存位，由任务端 W1C）
///   PIO 事件（CMD_COMPLETE / BUF_RD_READY / BUF_WR_READY）由 wait 函数 Phase 1 直接轮询。
pub fn sdhci_irq_handler(_irq: usize) {
    SDHCI_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);

    let base = SDHCI_IRQ_STATE.base.load(Ordering::Acquire);
    if base == 0 {
        return;
    }

    let status = mmio_read::<u32>(base + SDHCI_INT_STATUS_NORM as usize);
    if status == 0 {
        return;
    }

    let norm = status as u16;
    SDHCI_LAST_NORM.store(norm, Ordering::Relaxed);

    if norm & NORM_INT_CARD_INT != 0 {
        SDHCI_CARD_INT_COUNT.fetch_add(1, Ordering::Relaxed);
        mask_card_irq_raw(base, true);
        // SAFETY: CARD_INT 回调在初始化期间通过 register_card_irq_callback
        // 注册一次。单核：无并发写入。
        unsafe { SDHCI_IRQ_STATE.card_irq_callback.invoke() };
    }

    // 处理 XFER_COMPLETE：mask 信号，然后唤醒阻塞任务。
    // sticky 状态位不在此处清除——任务在唤醒后观察并 W1C 清除它。
    // 若在此清除会破坏唤醒条件并导致必然的 200ms 超时。
    // XFER_COMPLETE 通常仅在任务阻塞于 poll_int_status 时才在 SIG_EN 中
    // 使能（见 unmask_xfer_complete_signal），因此此路径在稳态时空闲。
    // 在 pre-check 快路径命中或超时退出后 SIG_EN 位可能短暂保持置位；
    // 由此产生的多余 ISR 重新 mask 并通知，无害。
    if norm & NORM_INT_XFER_COMPLETE != 0 {
        // Mask XFER_COMPLETE 信号以防重复触发
        rmw_norm_sig_en(base, 0, NORM_INT_XFER_COMPLETE);
        // 唤醒阻塞任务（状态位保持 sticky 供任务观察）
        // SAFETY: PIO 唤醒回调在初始化期间通过 register_pio_wake_callback
        // 注册一次。单核：无并发写入。
        unsafe { SDHCI_IRQ_STATE.pio_wake_callback.invoke() };
    }
}
```

---

## 文件：`drivers/blk/sdhci-cv1800/src/lib.rs`

- 冲突面：路径冲突。dev 侧（自分叉点以来）对该文件的唯一改动是主线 #1951 的纯路径迁移（0 行内容变化）；分支侧的改动为内容变更。变基中 git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对内容一致。
- 变更前路径：`components/sdhci-cv1800/src/lib.rs` @ e04580c5c
- 变更后路径：`drivers/blk/sdhci-cv1800/src/lib.rs` @ HEAD

> 说明："变更前"为该位置在分叉基线（`e04580c5c`，即 dev 未动其内容时的版本）上的整块源码；"变更后"为变基后当前分支的整块源码。

整块数量：修改 2 / 新增 5 / 删除 0

#### `//! 文件头注释`（修改）
变更前（第 1–12 行）：
```rust
//! CVI SoC (CV1800/SG2002) SDHCI 控制器驱动
//!
//! 职责:
//!   - SDHCI 标准寄存器操作 (CMD52/CMD53/PIO)
//!   - SDIO 卡枚举 (CMD5/CMD3/CMD7)
//!   - 中断处理 (ISR 仅 CARD_INT, PIO 事件直接轮询 INT_STATUS)
//!   - 时钟/电源/总线宽度配置
//!
//! 设计:
//!   - ISR: 仅处理 CARD_INT (WiFi 芯片通知有数据可读)
//!   - PIO: wait_* 方法直接轮询 INT_STATUS 寄存器，W1C 清除
//!   - 分离 ISR/PIO 消除竞态条件
```

变更后（第 1–18 行）：
```rust
//! CVI SoC (CV1800/SG2002) SDHCI 控制器驱动
//!
//! 职责:
//!   - SDHCI 标准寄存器操作 (CMD52/CMD53/PIO)
//!   - SDIO 卡枚举 (CMD5/CMD3/CMD7)
//!   - 中断处理 (ISR 处理 CARD_INT + XFER_COMPLETE)
//!   - 时钟/电源/总线宽度配置
//!
//! 设计:
//!   - ISR: 处理 CARD_INT (通知 WiFi 驱动) + XFER_COMPLETE (唤醒阻塞任务)
//!   - PIO: wait_* 方法 Phase 1 轮询 INT_STATUS, Phase 2 中断驱动等待
//!   - 丢唤醒防护: Phase 2 阻塞走 `SdhciDelay::block_timeout_until`，
//!     条件检查与任务入队在同一关中断临界区内衔接，配合 ISR 不消费的
//!     XFER_COMPLETE sticky 位——ISR 无论在锁内检查前触发（notify 落空、
//!     sticky 位被观察到）、检查后入队前（不可能，临界区关中断）还是
//!     入队后（notify 命中队列），事件都不会错过。
//!   - 单核串行化: ISR 与任务在单 hart 上执行。SIG_EN RMW 仍可被 ISR
//!     抢占，但事件由 sticky 位锁存（详见 irq.rs 模块文档）。
```

#### `const PHASE1_SPIN_ITERS: u32 = 1000;`（新增）
变更前：无（基线中不存在该整块）
变更后（第 37–38 行）：
```rust
/// Phase 1 快速自旋迭代次数（~50µs on C906 @1GHz）
const PHASE1_SPIN_ITERS: u32 = 1000;
```

#### `const PHASE2_STEP_MS: u64 = 10;`（新增）
变更前：无（基线中不存在该整块）
变更后（第 39–40 行）：
```rust
/// Phase 2 单次等待时长 (ms)
const PHASE2_STEP_MS: u64 = 10;
```

#### `const PHASE2_MAX_ITERS: u32 = 20;`（新增）
变更前：无（基线中不存在该整块）
变更后（第 41–42 行）：
```rust
/// Phase 2 最大迭代次数（总预算 = 20 × 10ms = 200ms）
const PHASE2_MAX_ITERS: u32 = 20;
```

#### `const PHASE2_WARN_AT: u32 = 10;`（新增）
变更前：无（基线中不存在该整块）
变更后（第 43–44 行）：
```rust
/// Phase 2 中段警告阈值（迭代次数）
const PHASE2_WARN_AT: u32 = 10;
```

#### `impl CviSdhci {`（修改）
变更前（第 72–549 行）：
```rust
impl CviSdhci {
    pub fn new(base_addr: usize) -> Self {
        Self {
            base: base_addr,
            rca: 0,
            vendor_id: 0,
            device_id: 0,
        }
    }

    #[inline(always)]
    fn read<T: Copy>(&self, off: u32) -> T {
        mmio_read::<T>(self.base + off as usize)
    }
    #[inline(always)]
    fn write<T: Copy>(&self, off: u32, val: T) {
        mmio_write::<T>(self.base + off as usize, val)
    }

    fn classify_error(err: u16) -> SdioError {
        if err & ERR_INT_CMD_CRC != 0 {
            log::error!("[SDHCI] CMD CRC error (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_DAT_CRC != 0 {
            log::error!("[SDHCI] DAT CRC error (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_CMD_TIMEOUT != 0 {
            log::error!("[SDHCI] CMD timeout (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_DAT_TIMEOUT != 0 {
            log::error!("[SDHCI] DAT timeout (err_sts=0x{:04x})", err);
        }
        match err {
            e if e & (ERR_INT_CMD_CRC | ERR_INT_DAT_CRC) != 0 => SdioError::CrcError,
            e if e & (ERR_INT_CMD_TIMEOUT | ERR_INT_DAT_TIMEOUT) != 0 => SdioError::Timeout,
            _ => SdioError::IoError,
        }
    }

    /// 直接轮询 INT_STATUS_NORM，等待指定 bit 置位后 W1C 清除
    ///
    /// 同时检测 Error 中断：如果 ERROR bit (bit 15) 置位，
    /// 读取 ERR_STATUS 并 W1C 清除所有状态位，然后返回错误。
    fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
        // Phase 1: 快速自旋
        for _ in 0..1000 {
            let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
            if norm & NORM_INT_ERROR != 0 {
                let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
                self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
                self.write::<u16>(SDHCI_INT_STATUS_NORM, norm);
                self.reset_dat_line();
                return Err(Self::classify_error(err));
            }
            if norm & bit != 0 {
                self.write::<u16>(SDHCI_INT_STATUS_NORM, bit);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        // Phase 2: 协作式等待
        for i in 0..200_000 {
            let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
            if norm & NORM_INT_ERROR != 0 {
                let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
                self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
                self.write::<u16>(SDHCI_INT_STATUS_NORM, norm);
                self.reset_dat_line();
                return Err(Self::classify_error(err));
            }
            if norm & bit != 0 {
                self.write::<u16>(SDHCI_INT_STATUS_NORM, bit);
                return Ok(());
            }
            if i == 100_000 {
                let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
                log::warn!(
                    "[SDHCI] poll_int mid-timeout: bit=0x{:04x} PRES=0x{:08x} INT_STS=0x{:04x}",
                    bit,
                    pres,
                    norm
                );
            }
            crate::runtime::delay().yield_now();
        }
        let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
        let sts = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        log::error!(
            "[SDHCI] poll_int_status timeout: bit=0x{:04x} PRES=0x{:08x} INT_STS=0x{:04x}",
            bit,
            pres,
            sts
        );
        // 超时后总线可能仍处于 DAT-busy(PRES bit1 DATA_INHIBIT 置位),若不复位
        // DAT 线状态机,后续任何数据命令的 wait_data_idle 都会一直超时,整条 SDIO
        // 总线被焊死(连 WiFi 模式切回 AP 也起不来)。这里对齐错误中断分支:清残留
        // INT_STATUS 并复位 DAT 线,让总线能从一次读超时中恢复。
        self.write::<u16>(SDHCI_INT_STATUS_NORM, sts);
        self.reset_dat_line();
        Err(SdioError::Timeout)
    }

    fn wait_cmd_complete(&self) -> Result<u32, SdioError> {
        self.poll_int_status(NORM_INT_CMD_COMPLETE)?;
        Ok(self.read::<u32>(SDHCI_RESPONSE))
    }

    fn wait_buffer_read_ready(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_BUF_RD_READY)
    }

    fn wait_buffer_write_ready(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_BUF_WR_READY)
    }

    fn wait_transfer_complete(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_XFER_COMPLETE)
    }

    /// 等待 CMD 线空闲 (仅检查 CMD_INHIBIT)
    fn wait_cmd_idle(&self) -> Result<(), SdioError> {
        for _ in 0..CMD_RESPONSE_TIMEOUT {
            if self.read::<u32>(SDHCI_PRESENT_STATE) & SDHCI_CMD_INHIBIT == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    /// 等待 CMD 和 DAT 线都空闲 (数据命令前使用)
    fn wait_data_idle(&self) -> Result<(), SdioError> {
        for _ in 0..CMD_RESPONSE_TIMEOUT {
            if self.read::<u32>(SDHCI_PRESENT_STATE) & (SDHCI_CMD_INHIBIT | SDHCI_DATA_INHIBIT) == 0
            {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
        log::error!("[SDHCI] wait_data_idle timeout: PRES=0x{:08x}", pres);
        Err(SdioError::Timeout)
    }

    fn wait_clock_stable(&self) -> Result<(), SdioError> {
        for _ in 0..CLOCK_STABLE_TIMEOUT {
            if self.read::<u16>(SDHCI_CLOCK_CONTROL) & CC_INT_CLK_STABLE != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    fn wait_reset_complete(&self) -> Result<(), SdioError> {
        for _ in 0..RESET_TIMEOUT {
            if self.read::<u8>(SDHCI_SOFTWARE_RESET) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    fn reset_dat_line(&self) {
        self.write::<u8>(SDHCI_SOFTWARE_RESET, SWRST_DAT_LINE);
        for _ in 0..RESET_TIMEOUT {
            if self.read::<u8>(SDHCI_SOFTWARE_RESET) & SWRST_DAT_LINE == 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }

    /// Clear stale INT_STATUS bits (W1C clear all set bits)
    fn clear_stale_status(&self) {
        let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        if norm != 0 {
            if norm & NORM_INT_ERROR != 0 {
                let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
                if err != 0 {
                    self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
                }
            }
            self.write::<u16>(SDHCI_INT_STATUS_NORM, norm);
        }
    }

    /// Clear DAT state machine and stale INT_STATUS for first data transfer.
    pub fn prepare_first_data_xfer(&self) {
        self.write::<u16>(SDHCI_INT_STATUS_NORM, 0xFFFF);
        self.write::<u16>(SDHCI_INT_STATUS_ERR, 0xFFFF);
        self.reset_dat_line();
        log::debug!("[SDHCI] DAT line reset + INT_STATUS cleared for first data xfer");
    }

    /// SD 命令 (非数据命令: CMD0/3/5/7/52)
    fn send_cmd(&self, cmd_idx: u8, arg: u32) -> Result<u32, SdioError> {
        self.wait_cmd_idle()?;
        self.clear_stale_status();

        self.write::<u32>(SDHCI_ARGUMENT, arg);
        let flags = match cmd_idx {
            0 => CMD_RESP_NONE,
            3 => CMD_FLAGS_R5, // R6 与 R5 标志相同
            5 => CMD_FLAGS_R4,
            7 => CMD_FLAGS_R1B,
            52 => CMD_FLAGS_R5,
            _ => return Err(SdioError::Unsupported),
        };

        self.write::<u16>(
            SDHCI_COMMAND,
            (cmd_idx as u16) << CMD_INDEX_SHIFT as u16 | flags,
        );
        self.wait_cmd_complete()
    }

    fn check_r5_response(&self, resp: u32) -> Result<u8, SdioError> {
        if resp & R5_COM_CRC_ERROR != 0 {
            log::error!("[SDHCI] R5 CRC error, resp=0x{:08x}", resp);
            return Err(SdioError::CrcError);
        }
        if resp & (R5_ILLEGAL_COMMAND | R5_FUNCTION_NUMBER | R5_OUT_OF_RANGE) != 0 {
            log::error!("[SDHCI] R5 cmd/func/range error, resp=0x{:08x}", resp);
            return Err(SdioError::IoError);
        }
        if resp & R5_ERROR != 0 {
            log::error!("[SDHCI] R5 general error, resp=0x{:08x}", resp);
            return Err(SdioError::IoError);
        }
        Ok((resp & R5_DATA_MASK) as u8)
    }

    /// CMD52
    fn cmd52(&self, func: u8, addr: u32, flags: u32, val: u8) -> Result<u8, SdioError> {
        if addr > SDIO_ADDR_MASK {
            return Err(SdioError::Unsupported);
        }
        let arg =
            flags | ((func as u32 & 0x07) << 28) | ((addr & SDIO_ADDR_MASK) << 9) | val as u32;
        let resp = self.send_cmd(52, arg)?;
        self.check_r5_response(resp)
    }

    fn cmd52_read(&self, func: u8, addr: u32) -> Result<u8, SdioError> {
        self.cmd52(func, addr, 0, 0)
    }

    fn cmd52_write(&self, func: u8, addr: u32, val: u8) -> Result<(), SdioError> {
        self.cmd52(func, addr, CMD52_RW_FLAG, val)?;
        Ok(())
    }

    /// CMD53 数据传输设置
    ///
    /// 关键改进:
    ///   - 检查 DATA_INHIBIT (确保前一次数据传输完成)
    ///   - TRANSFER_MODE + COMMAND 作为 32-bit 原子写入
    ///   - BLOCK_SIZE 寄存器设置 SDMA boundary 字段
    #[allow(clippy::too_many_arguments)]
    fn cmd53_xfer(
        &self,
        func: u8,
        addr: u32,
        write: bool,
        inc_addr: bool,
        block_size: u16,
        use_block: bool,
        len: usize,
    ) -> Result<(u16, u16), SdioError> {
        if addr > SDIO_ADDR_MASK || len == 0 {
            return Err(SdioError::Unsupported);
        }

        let (blk_mode, count, blk_sz) = if use_block && block_size > 0 {
            let n = len / block_size as usize;
            if n == 0 || !len.is_multiple_of(block_size as usize) {
                return Err(SdioError::Unsupported);
            }
            (true, n, block_size)
        } else {
            if len > SDIO_DEFAULT_BLOCK_SIZE as usize {
                return Err(SdioError::Unsupported);
            }
            (
                false,
                if len == SDIO_DEFAULT_BLOCK_SIZE as usize {
                    0
                } else {
                    len
                },
                len as u16,
            )
        };

        let mut arg =
            ((func as u32 & 0x07) << 28) | ((addr & SDIO_ADDR_MASK) << 9) | (count as u32 & 0x1FF);
        if write {
            arg |= CMD53_RW_FLAG;
        }
        if blk_mode {
            arg |= CMD53_BLOCK_MODE;
        }
        if inc_addr {
            arg |= CMD53_OP_CODE_INC;
        }

        let xfer_blocks = if blk_mode { count as u16 } else { 1 };

        // 等待 CMD 和 DAT 线都空闲
        self.wait_data_idle()?;
        self.clear_stale_status();

        // BLOCK_SIZE: bits[11:0]=block size, bits[14:12]=SDMA boundary (0x7=512K)
        self.write::<u16>(SDHCI_BLOCK_SIZE, blk_sz | SDHCI_SDMA_BOUNDARY_512K);
        self.write::<u16>(SDHCI_BLOCK_COUNT, xfer_blocks);

        // TRANSFER_MODE (offset 0x0C) + COMMAND (offset 0x0E) 作为 32-bit 原子写入
        let tm = if blk_mode {
            TM_MULTI_BLOCK | TM_BLK_CNT_EN
        } else {
            0
        } | if !write { TM_DATA_DIR_READ } else { 0 };

        let cmd_val = (53u16) << CMD_INDEX_SHIFT as u16 | CMD_FLAGS_R5_DATA;
        self.write::<u32>(SDHCI_ARGUMENT, arg);
        self.write::<u32>(SDHCI_TRANSFER_MODE, ((cmd_val as u32) << 16) | (tm as u32));

        self.wait_cmd_complete()?;
        Ok((blk_sz, xfer_blocks))
    }

    fn cmd53_read_fixed(
        &self,
        func: u8,
        addr: u32,
        buf: &mut [u8],
        blk_sz: u16,
        use_blk: bool,
    ) -> Result<(), SdioError> {
        let (bs, nb) = self.cmd53_xfer(func, addr, false, false, blk_sz, use_blk, buf.len())?;
        self.pio_read(buf, bs, nb)?;
        self.wait_transfer_complete()
    }

    fn cmd53_write_fixed(
        &self,
        func: u8,
        addr: u32,
        buf: &[u8],
        blk_sz: u16,
        use_blk: bool,
    ) -> Result<(), SdioError> {
        let (bs, nb) = self.cmd53_xfer(func, addr, true, false, blk_sz, use_blk, buf.len())?;
        self.pio_write(buf, bs, nb)?;
        self.wait_transfer_complete()
    }

    /// PIO 读取: 逐块等待 Buffer Read Ready → 读取 Buffer Data Port
    fn pio_read(&self, buf: &mut [u8], block_size: u16, nblocks: u16) -> Result<(), SdioError> {
        let mut offset = 0;

        for _ in 0..nblocks {
            self.wait_buffer_read_ready()?;

            let words = (block_size as usize).div_ceil(4);
            for _ in 0..words {
                let data = self.read::<u32>(SDHCI_BUFFER);
                let byte_offset = data.to_le_bytes();
                let remaining = buf.len() - offset;
                let copy_len = core::cmp::min(4, remaining);
                buf[offset..offset + copy_len].copy_from_slice(&byte_offset[..copy_len]);
                offset += copy_len;
            }
        }

        Ok(())
    }

    /// PIO 写入: 逐块等待 Buffer Write Ready → 写入 Buffer Data Port
    fn pio_write(&self, buf: &[u8], block_size: u16, nblocks: u16) -> Result<(), SdioError> {
        let mut offset = 0;

        for _ in 0..nblocks {
            self.wait_buffer_write_ready()?;

            let words = (block_size as usize).div_ceil(4);
            for _ in 0..words {
                let mut data: [u8; 4] = [0; 4];
                let remaining = buf.len() - offset;
                let copy_len = core::cmp::min(4, remaining);
                data[..copy_len].copy_from_slice(&buf[offset..offset + copy_len]);
                let word = u32::from_le_bytes(data);
                self.write::<u32>(SDHCI_BUFFER, word);
                offset += copy_len;
            }
        }

        Ok(())
    }

    /// 读取 CIS 指针 (3 字节, little-endian)
    fn read_cis_ptr(&self, func: u8) -> Result<u32, SdioError> {
        let base = if func == 0 {
            CCCR_CIS_POINTER
        } else {
            fbr_base(func) + FBR_CIS_PTR_OFFSET
        };
        let b0 = self.cmd52_read(0, base)? as u32;
        let b1 = self.cmd52_read(0, base + 1)? as u32;
        let b2 = self.cmd52_read(0, base + 2)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16))
    }

    /// 遍历 CIS tuple 链，查找 CISTPL_MANFID，返回 (vendor_id, device_id)
    fn read_manfid_from_cis(&self, func: u8) -> Result<(u16, u16), SdioError> {
        let mut addr = self.read_cis_ptr(func)?;
        for _ in 0..256 {
            let tuple_code = self.cmd52_read(0, addr)?;
            if tuple_code == CISTPL_END {
                break;
            }
            if tuple_code == CISTPL_NULL {
                addr += 1;
                continue;
            }
            let tuple_link = self.cmd52_read(0, addr + 1)? as u32;
            if tuple_code == CISTPL_MANFID && tuple_link >= 4 {
                let v0 = self.cmd52_read(0, addr + 2)? as u16;
                let v1 = self.cmd52_read(0, addr + 3)? as u16;
                let v2 = self.cmd52_read(0, addr + 4)? as u16;
                let v3 = self.cmd52_read(0, addr + 5)? as u16;
                return Ok((v0 | (v1 << 8), v2 | (v3 << 8)));
            }
            addr += 2 + tuple_link;
        }

        Err(SdioError::Unsupported)
    }

    // ========== SDIO 初始化辅助函数 ==========

    /// SDHCI 控制器软件复位
    fn controller_reset(&self) -> Result<(), SdioError> {
        self.write::<u8>(SDHCI_SOFTWARE_RESET, SWRST_ALL);
        self.wait_reset_complete()
    }

    /// 设置卡检测覆写（WiFi 模块无物理 CD 引脚）
    fn setup_card_detect(&self) -> Result<(), SdioError> {
        let hc = self.read::<u8>(SDHCI_HOST_CONTROL);
        self.write::<u8>(SDHCI_HOST_CONTROL, hc | HC_CARD_DET_TEST | HC_CARD_DET_SEL);
        Ok(())
    }

    /// 上电 3.3V（必须在启动时钟之前）
    fn power_on(&self) -> Result<(), SdioError> {
        self.write::<u8>(SDHCI_POWER_CONTROL, POWER_330V_ON);
        Ok(())
    }

    /// 设置初始低速时钟 400KHz
    fn setup_initial_clock(&self) -> Result<(), SdioError> {
        self.set_clock(400_000)
    }

    /// 使能中断状态位 + CARD_INT 信号
    fn enable_interrupts_irq(&self) -> Result<(), SdioError> {
        irq::irq_state_init(self.base);
        // Status Enable: 使能所有状态位 (用于 poll_int_status 轮询)
        self.write::<u16>(SDHCI_NORM_INT_STS_EN, NORM_INT_ENABLE_MASK);
        self.write::<u16>(SDHCI_ERR_INT_STS_EN, ERR_INT_ENABLE_MASK);
        // Signal Enable: 仅使能 CARD_INT (ISR 只处理 CARD_INT)
        irq::enable_irq_signals();
        Ok(())
    }
}
```

变更后（第 87–650 行）：
```rust
impl CviSdhci {
    pub fn new(base_addr: usize) -> Self {
        Self {
            base: base_addr,
            rca: 0,
            vendor_id: 0,
            device_id: 0,
        }
    }

    #[inline(always)]
    fn read<T: Copy>(&self, off: u32) -> T {
        mmio_read::<T>(self.base + off as usize)
    }
    #[inline(always)]
    fn write<T: Copy>(&self, off: u32, val: T) {
        mmio_write::<T>(self.base + off as usize, val)
    }

    fn classify_error(err: u16) -> SdioError {
        if err & ERR_INT_CMD_CRC != 0 {
            log::error!("[SDHCI] CMD CRC error (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_DAT_CRC != 0 {
            log::error!("[SDHCI] DAT CRC error (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_CMD_TIMEOUT != 0 {
            log::error!("[SDHCI] CMD timeout (err_sts=0x{:04x})", err);
        }
        if err & ERR_INT_DAT_TIMEOUT != 0 {
            log::error!("[SDHCI] DAT timeout (err_sts=0x{:04x})", err);
        }
        match err {
            e if e & (ERR_INT_CMD_CRC | ERR_INT_DAT_CRC) != 0 => SdioError::CrcError,
            e if e & (ERR_INT_CMD_TIMEOUT | ERR_INT_DAT_TIMEOUT) != 0 => SdioError::Timeout,
            _ => SdioError::IoError,
        }
    }

    /// 仅 W1C 清除 INT_STATUS_NORM 中指定的位。
    /// 绝不清除 CARD_INT——该位由 ISR/mask 协议独占管理。
    fn clear_int_status_norm(&self, bits: u16) {
        self.write::<u16>(SDHCI_INT_STATUS_NORM, bits);
    }

    /// 轮询 INT_STATUS 一次：检查错误或目标位，命中时消费。
    /// 返回：
    /// - `Some(Ok(()))` 目标位置位（W1C 清除）
    /// - `Some(Err(...))` 检测到错误中断（W1C 清除 + DAT 复位）
    /// - `None` 两个条件均未满足（继续轮询/等待）
    fn poll_status_once(&self, bit: u16) -> Option<Result<(), SdioError>> {
        let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        if norm & NORM_INT_ERROR != 0 {
            let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
            self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
            // 选择性清除：错误位 + 等待位 + XFER_COMPLETE。
            // XFER_COMPLETE 可能与错误同时被置位（如数据阶段完成后 DAT 错误）。
            // 在此消费可防止 stale bit 泄漏至下一传输的 wait_transfer_complete
            // 导致虚假过早成功。CARD_INT 有意保留（ISR/mask 协议）。
            self.clear_int_status_norm(NORM_INT_ERROR | bit | NORM_INT_XFER_COMPLETE);
            self.reset_dat_line();
            return Some(Err(Self::classify_error(err)));
        }
        if norm & bit != 0 {
            self.clear_int_status_norm(bit);
            return Some(Ok(()));
        }
        None
    }

    /// 轮询 INT_STATUS_NORM，等待指定 bit 置位后选择性 W1C 清除。
    ///
    /// Phase 1: 快速自旋 (PHASE1_SPIN_ITERS 次, ~50µs)
    /// Phase 2: XFER_COMPLETE 走硬件中断驱动等待；其他位使用 10ms 睡眠轮询
    ///
    /// 同时检测 Error 中断：如果 ERROR bit (bit 15) 置位，
    /// 读取 ERR_STATUS，选择性 W1C 清除错误位 + 当前等待位（保留 CARD_INT），
    /// 然后复位 DAT 线并返回错误。
    fn poll_int_status(&self, bit: u16) -> Result<(), SdioError> {
        // 在进入 Phase 1 自旋循环前排空存储缓冲区。
        // 若无此栅栏，待处理的 MMIO 写（如 pio_write 的 128 次 SDHCI_BUFFER
        // 写入）可能仍排在 CPU 存储缓冲区中，而此时后续的
        // mmio_read(INT_STATUS_NORM) 循环已开始。读取与排空写入在 SDHCI
        // 总线上竞争，延迟硬件实际接收数据并置位 BUF_WR_READY/CMD_COMPLETE。
        // Phase 1 的 1000 次迭代窗口（约 50µs）可能在状态位可见前过期，
        // 导致落入 10ms Phase 2 延迟。此处单一栅栏保证轮询开始前存储缓冲区
        // 已空——与旧 yield_now 忙等待方案中任务切换隐含栅栏（mret）效果相同。
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Phase 1: 快速自旋
        for _ in 0..PHASE1_SPIN_ITERS {
            if let Some(result) = self.poll_status_once(bit) {
                return result;
            }
            core::hint::spin_loop();
        }
        // Phase 2: XFER_COMPLETE 走硬件中断驱动等待；其他位走纯超时睡眠
        // （不经过 WaitQueue——ISR 不会为这些位发 notify，经过 WQ 是无效开销）
        let use_irq = bit == NORM_INT_XFER_COMPLETE;
        if !use_irq {
            log::trace!("[SDHCI] poll_int Phase-2 fallback: bit=0x{:04x}", bit);
        }
        let mut timeout_count: u32 = 0;
        for i in 0..PHASE2_MAX_ITERS {
            // 阻塞前先检查状态寄存器（快路径）。此 pre-check 不能单独关闭
            // 丢唤醒窗口——真正的防护是下方的 block_timeout_until 协议：
            // 条件检查与任务入队在同一关中断临界区内衔接，unmask 与入队
            // 之间的 ISR 事件由 XFER_COMPLETE sticky 位保留并被锁内条件
            // 检查观察到（见 SdhciDelay 契约）。
            if let Some(result) = self.poll_status_once(bit) {
                return result;
            }

            if i == PHASE2_WARN_AT {
                let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
                let sts = self.read::<u16>(SDHCI_INT_STATUS_NORM);
                log::warn!(
                    "[SDHCI] poll_int mid-timeout: bit=0x{:04x} PRES=0x{:08x} INT_STS=0x{:04x} \
                     timeouts={}",
                    bit,
                    pres,
                    sts,
                    timeout_count
                );
            }

            if use_irq {
                irq::unmask_xfer_complete_signal();
                // 条件等待：ISR 若在 unmask 后、锁内检查前触发，其 notify
                // 落空（队列为空），但它 latch 的 sticky 位会被胶水层锁内
                // 条件检查立即观察到，任务无需等满超时。错误位也纳入条件：
                // 错误不产生中断（SIG_EN 不含 error 位），锁内检查前已锁存
                // 的错误可即时返回；睡期中段到达的错误仍由 10ms 超时后的
                // post-wake 重检查出，与旧实现时延一致。
                let timed_out =
                    crate::runtime::delay().block_timeout_until(PHASE2_STEP_MS, &|| {
                        let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
                        norm & (bit | NORM_INT_ERROR) != 0
                    });
                // 返回值无需驱动分支——post-wake 重检无条件执行，覆盖两种
                // 返回路径。仅在超时（10ms 退化路径）时留观测信号。
                if timed_out {
                    log::trace!(
                        "[SDHCI] poll_int Phase-2 IRQ wait timed out: bit=0x{:04x}",
                        bit
                    );
                }
            } else {
                // 非 XFER 位：直接 sleep，不经过 WaitQueue——
                // ISR 只对 XFER_COMPLETE 发 notify，经过 WQ 是无效开销。
                crate::runtime::delay().delay_ms(PHASE2_STEP_MS);
            }
            timeout_count += 1;

            // 被唤醒后检查状态寄存器
            if let Some(result) = self.poll_status_once(bit) {
                return result;
            }
        }
        let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
        let sts = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        log::error!(
            "[SDHCI] poll_int_status timeout: bit=0x{:04x} PRES=0x{:08x} INT_STS=0x{:04x} \
             timeouts={}",
            bit,
            pres,
            sts,
            timeout_count
        );
        // 超时后总线可能仍处于 DAT-busy(PRES bit1 DATA_INHIBIT 置位),若不复位
        // DAT 线状态机,后续任何数据命令的 wait_data_idle 都会一直超时,整条 SDIO
        // 总线被焊死(连 WiFi 模式切回 AP 也起不来)。这里对齐错误中断分支:选择性
        // 清除错误位 + 当前等待位 + XFER_COMPLETE（防止 stale bit 泄漏到下一传输），
        // 保留 CARD_INT，然后复位 DAT 线。
        self.clear_int_status_norm(NORM_INT_ERROR | bit | NORM_INT_XFER_COMPLETE);
        self.reset_dat_line();
        Err(SdioError::Timeout)
    }

    fn wait_cmd_complete(&self) -> Result<u32, SdioError> {
        self.poll_int_status(NORM_INT_CMD_COMPLETE)?;
        Ok(self.read::<u32>(SDHCI_RESPONSE))
    }

    fn wait_buffer_read_ready(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_BUF_RD_READY)
    }

    fn wait_buffer_write_ready(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_BUF_WR_READY)
    }

    fn wait_transfer_complete(&self) -> Result<(), SdioError> {
        self.poll_int_status(NORM_INT_XFER_COMPLETE)
    }

    /// 等待 CMD 线空闲 (仅检查 CMD_INHIBIT)
    fn wait_cmd_idle(&self) -> Result<(), SdioError> {
        for _ in 0..CMD_RESPONSE_TIMEOUT {
            if self.read::<u32>(SDHCI_PRESENT_STATE) & SDHCI_CMD_INHIBIT == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    /// 等待 CMD 和 DAT 线都空闲 (数据命令前使用)
    fn wait_data_idle(&self) -> Result<(), SdioError> {
        for _ in 0..CMD_RESPONSE_TIMEOUT {
            if self.read::<u32>(SDHCI_PRESENT_STATE) & (SDHCI_CMD_INHIBIT | SDHCI_DATA_INHIBIT) == 0
            {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        let pres = self.read::<u32>(SDHCI_PRESENT_STATE);
        log::error!("[SDHCI] wait_data_idle timeout: PRES=0x{:08x}", pres);
        Err(SdioError::Timeout)
    }

    fn wait_clock_stable(&self) -> Result<(), SdioError> {
        for _ in 0..CLOCK_STABLE_TIMEOUT {
            if self.read::<u16>(SDHCI_CLOCK_CONTROL) & CC_INT_CLK_STABLE != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    fn wait_reset_complete(&self) -> Result<(), SdioError> {
        for _ in 0..RESET_TIMEOUT {
            if self.read::<u8>(SDHCI_SOFTWARE_RESET) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(SdioError::Timeout)
    }

    fn reset_dat_line(&self) {
        self.write::<u8>(SDHCI_SOFTWARE_RESET, SWRST_DAT_LINE);
        for _ in 0..RESET_TIMEOUT {
            if self.read::<u8>(SDHCI_SOFTWARE_RESET) & SWRST_DAT_LINE == 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }

    /// 在启动命令前清除残留的 INT_STATUS 位。
    ///
    /// 使用选择性 W1C：保留 XFER_COMPLETE（可能被阻塞在
    /// `poll_int_status` Phase 2 的任务消费）。ISR 同样不清除
    /// XFER_COMPLETE——仅等待任务的 recheck 在 `poll_int_status`
    /// 中可消费它。
    fn clear_stale_status(&self) {
        let norm = self.read::<u16>(SDHCI_INT_STATUS_NORM);
        // Mask 掉 XFER_COMPLETE——它属于可能阻塞的 PIO waiter，
        // 不得被命令路径破坏。
        let clearable = norm & !NORM_INT_XFER_COMPLETE;
        if clearable != 0 {
            if clearable & NORM_INT_ERROR != 0 {
                let err = self.read::<u16>(SDHCI_INT_STATUS_ERR);
                if err != 0 {
                    self.write::<u16>(SDHCI_INT_STATUS_ERR, err);
                }
            }
            self.write::<u16>(SDHCI_INT_STATUS_NORM, clearable);
        }
    }

    /// Clear DAT state machine and stale INT_STATUS for first data transfer.
    pub fn prepare_first_data_xfer(&self) {
        self.write::<u16>(SDHCI_INT_STATUS_NORM, 0xFFFF);
        self.write::<u16>(SDHCI_INT_STATUS_ERR, 0xFFFF);
        self.reset_dat_line();
        log::debug!("[SDHCI] DAT line reset + INT_STATUS cleared for first data xfer");
    }

    /// SD 命令 (非数据命令: CMD0/3/5/7/52)
    fn send_cmd(&self, cmd_idx: u8, arg: u32) -> Result<u32, SdioError> {
        self.wait_cmd_idle()?;
        self.clear_stale_status();

        self.write::<u32>(SDHCI_ARGUMENT, arg);
        let flags = match cmd_idx {
            0 => CMD_RESP_NONE,
            3 => CMD_FLAGS_R5, // R6 与 R5 标志相同
            5 => CMD_FLAGS_R4,
            7 => CMD_FLAGS_R1B,
            52 => CMD_FLAGS_R5,
            _ => return Err(SdioError::Unsupported),
        };

        self.write::<u16>(
            SDHCI_COMMAND,
            (cmd_idx as u16) << CMD_INDEX_SHIFT as u16 | flags,
        );
        self.wait_cmd_complete()
    }

    fn check_r5_response(&self, resp: u32) -> Result<u8, SdioError> {
        if resp & R5_COM_CRC_ERROR != 0 {
            log::error!("[SDHCI] R5 CRC error, resp=0x{:08x}", resp);
            return Err(SdioError::CrcError);
        }
        if resp & (R5_ILLEGAL_COMMAND | R5_FUNCTION_NUMBER | R5_OUT_OF_RANGE) != 0 {
            log::error!("[SDHCI] R5 cmd/func/range error, resp=0x{:08x}", resp);
            return Err(SdioError::IoError);
        }
        if resp & R5_ERROR != 0 {
            log::error!("[SDHCI] R5 general error, resp=0x{:08x}", resp);
            return Err(SdioError::IoError);
        }
        Ok((resp & R5_DATA_MASK) as u8)
    }

    /// CMD52
    fn cmd52(&self, func: u8, addr: u32, flags: u32, val: u8) -> Result<u8, SdioError> {
        if addr > SDIO_ADDR_MASK {
            return Err(SdioError::Unsupported);
        }
        let arg =
            flags | ((func as u32 & 0x07) << 28) | ((addr & SDIO_ADDR_MASK) << 9) | val as u32;
        let resp = self.send_cmd(52, arg)?;
        self.check_r5_response(resp)
    }

    fn cmd52_read(&self, func: u8, addr: u32) -> Result<u8, SdioError> {
        self.cmd52(func, addr, 0, 0)
    }

    fn cmd52_write(&self, func: u8, addr: u32, val: u8) -> Result<(), SdioError> {
        self.cmd52(func, addr, CMD52_RW_FLAG, val)?;
        Ok(())
    }

    /// CMD53 数据传输设置
    ///
    /// 关键改进:
    ///   - 检查 DATA_INHIBIT (确保前一次数据传输完成)
    ///   - TRANSFER_MODE + COMMAND 作为 32-bit 原子写入
    ///   - BLOCK_SIZE 寄存器设置 SDMA boundary 字段
    #[allow(clippy::too_many_arguments)]
    fn cmd53_xfer(
        &self,
        func: u8,
        addr: u32,
        write: bool,
        inc_addr: bool,
        block_size: u16,
        use_block: bool,
        len: usize,
    ) -> Result<(u16, u16), SdioError> {
        if addr > SDIO_ADDR_MASK || len == 0 {
            return Err(SdioError::Unsupported);
        }

        let (blk_mode, count, blk_sz) = if use_block && block_size > 0 {
            let n = len / block_size as usize;
            if n == 0 || !len.is_multiple_of(block_size as usize) {
                return Err(SdioError::Unsupported);
            }
            (true, n, block_size)
        } else {
            if len > SDIO_DEFAULT_BLOCK_SIZE as usize {
                return Err(SdioError::Unsupported);
            }
            (
                false,
                if len == SDIO_DEFAULT_BLOCK_SIZE as usize {
                    0
                } else {
                    len
                },
                len as u16,
            )
        };

        let mut arg =
            ((func as u32 & 0x07) << 28) | ((addr & SDIO_ADDR_MASK) << 9) | (count as u32 & 0x1FF);
        if write {
            arg |= CMD53_RW_FLAG;
        }
        if blk_mode {
            arg |= CMD53_BLOCK_MODE;
        }
        if inc_addr {
            arg |= CMD53_OP_CODE_INC;
        }

        let xfer_blocks = if blk_mode { count as u16 } else { 1 };

        // 等待 CMD 和 DAT 线都空闲
        self.wait_data_idle()?;
        self.clear_stale_status();

        // BLOCK_SIZE: bits[11:0]=block size, bits[14:12]=SDMA boundary (0x7=512K)
        self.write::<u16>(SDHCI_BLOCK_SIZE, blk_sz | SDHCI_SDMA_BOUNDARY_512K);
        self.write::<u16>(SDHCI_BLOCK_COUNT, xfer_blocks);

        // TRANSFER_MODE (offset 0x0C) + COMMAND (offset 0x0E) 作为 32-bit 原子写入
        let tm = if blk_mode {
            TM_MULTI_BLOCK | TM_BLK_CNT_EN
        } else {
            0
        } | if !write { TM_DATA_DIR_READ } else { 0 };

        let cmd_val = (53u16) << CMD_INDEX_SHIFT as u16 | CMD_FLAGS_R5_DATA;
        self.write::<u32>(SDHCI_ARGUMENT, arg);
        self.write::<u32>(SDHCI_TRANSFER_MODE, ((cmd_val as u32) << 16) | (tm as u32));

        self.wait_cmd_complete()?;
        Ok((blk_sz, xfer_blocks))
    }

    fn cmd53_read_fixed(
        &self,
        func: u8,
        addr: u32,
        buf: &mut [u8],
        blk_sz: u16,
        use_blk: bool,
    ) -> Result<(), SdioError> {
        let (bs, nb) = self.cmd53_xfer(func, addr, false, false, blk_sz, use_blk, buf.len())?;
        self.pio_read(buf, bs, nb)?;
        self.wait_transfer_complete()
    }

    fn cmd53_write_fixed(
        &self,
        func: u8,
        addr: u32,
        buf: &[u8],
        blk_sz: u16,
        use_blk: bool,
    ) -> Result<(), SdioError> {
        let (bs, nb) = self.cmd53_xfer(func, addr, true, false, blk_sz, use_blk, buf.len())?;
        self.pio_write(buf, bs, nb)?;
        self.wait_transfer_complete()
    }

    /// PIO 读取: 逐块等待 Buffer Read Ready → 读取 Buffer Data Port
    fn pio_read(&self, buf: &mut [u8], block_size: u16, nblocks: u16) -> Result<(), SdioError> {
        let mut offset = 0;

        for _ in 0..nblocks {
            self.wait_buffer_read_ready()?;

            let words = (block_size as usize).div_ceil(4);
            for _ in 0..words {
                let data = self.read::<u32>(SDHCI_BUFFER);
                let byte_offset = data.to_le_bytes();
                let remaining = buf.len() - offset;
                let copy_len = core::cmp::min(4, remaining);
                buf[offset..offset + copy_len].copy_from_slice(&byte_offset[..copy_len]);
                offset += copy_len;
            }
        }

        Ok(())
    }

    /// PIO 写入: 逐块等待 Buffer Write Ready → 写入 Buffer Data Port
    fn pio_write(&self, buf: &[u8], block_size: u16, nblocks: u16) -> Result<(), SdioError> {
        let mut offset = 0;

        for _ in 0..nblocks {
            self.wait_buffer_write_ready()?;

            let words = (block_size as usize).div_ceil(4);
            for _ in 0..words {
                let mut data: [u8; 4] = [0; 4];
                let remaining = buf.len() - offset;
                let copy_len = core::cmp::min(4, remaining);
                data[..copy_len].copy_from_slice(&buf[offset..offset + copy_len]);
                let word = u32::from_le_bytes(data);
                self.write::<u32>(SDHCI_BUFFER, word);
                offset += copy_len;
            }
        }

        Ok(())
    }

    /// 读取 CIS 指针 (3 字节, little-endian)
    fn read_cis_ptr(&self, func: u8) -> Result<u32, SdioError> {
        let base = if func == 0 {
            CCCR_CIS_POINTER
        } else {
            fbr_base(func) + FBR_CIS_PTR_OFFSET
        };
        let b0 = self.cmd52_read(0, base)? as u32;
        let b1 = self.cmd52_read(0, base + 1)? as u32;
        let b2 = self.cmd52_read(0, base + 2)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16))
    }

    /// 遍历 CIS tuple 链，查找 CISTPL_MANFID，返回 (vendor_id, device_id)
    fn read_manfid_from_cis(&self, func: u8) -> Result<(u16, u16), SdioError> {
        let mut addr = self.read_cis_ptr(func)?;
        for _ in 0..256 {
            let tuple_code = self.cmd52_read(0, addr)?;
            if tuple_code == CISTPL_END {
                break;
            }
            if tuple_code == CISTPL_NULL {
                addr += 1;
                continue;
            }
            let tuple_link = self.cmd52_read(0, addr + 1)? as u32;
            if tuple_code == CISTPL_MANFID && tuple_link >= 4 {
                let v0 = self.cmd52_read(0, addr + 2)? as u16;
                let v1 = self.cmd52_read(0, addr + 3)? as u16;
                let v2 = self.cmd52_read(0, addr + 4)? as u16;
                let v3 = self.cmd52_read(0, addr + 5)? as u16;
                return Ok((v0 | (v1 << 8), v2 | (v3 << 8)));
            }
            addr += 2 + tuple_link;
        }

        Err(SdioError::Unsupported)
    }

    // ========== SDIO 初始化辅助函数 ==========

    /// SDHCI 控制器软件复位
    fn controller_reset(&self) -> Result<(), SdioError> {
        self.write::<u8>(SDHCI_SOFTWARE_RESET, SWRST_ALL);
        self.wait_reset_complete()
    }

    /// 设置卡检测覆写（WiFi 模块无物理 CD 引脚）
    fn setup_card_detect(&self) -> Result<(), SdioError> {
        let hc = self.read::<u8>(SDHCI_HOST_CONTROL);
        self.write::<u8>(SDHCI_HOST_CONTROL, hc | HC_CARD_DET_TEST | HC_CARD_DET_SEL);
        Ok(())
    }

    /// 上电 3.3V（必须在启动时钟之前）
    fn power_on(&self) -> Result<(), SdioError> {
        self.write::<u8>(SDHCI_POWER_CONTROL, POWER_330V_ON);
        Ok(())
    }

    /// 设置初始低速时钟 400KHz
    fn setup_initial_clock(&self) -> Result<(), SdioError> {
        self.set_clock(400_000)
    }

    /// 使能中断状态位 + CARD_INT 信号。
    /// XFER_COMPLETE 信号由 poll_int_status 阻塞前通过 unmask_xfer_complete_signal 动态启用。
    fn enable_interrupts_irq(&self) -> Result<(), SdioError> {
        irq::irq_state_init(self.base);
        // 状态使能：使能所有状态位（用于 poll_int_status 轮询）
        self.write::<u16>(SDHCI_NORM_INT_STS_EN, NORM_INT_ENABLE_MASK);
        self.write::<u16>(SDHCI_ERR_INT_STS_EN, ERR_INT_ENABLE_MASK);
        // 信号使能：仅使能 CARD_INT；XFER_COMPLETE 由 poll_int_status 动态 un-mask
        irq::enable_irq_signals();
        Ok(())
    }
}
```

#### `mod tests {`（新增）
变更前：无（基线中不存在该整块）
变更后（第 907–1060 行）：
```rust
#[cfg(test)]
mod tests {
    //! 丢唤醒协议回归验证。
    //!
    //! 全局状态注意：`set_delay` 与 `irq_state_init` 安装的是进程级全局
    //! provider/基地址，不同测试会相互覆盖，因此新增场景必须并入下面的
    //! 单一测试函数内顺序执行。
    //!
    //! 建模局限（记录在案）：
    //! - W1C 未建模：对 INT_STATUS 的写是直接覆写而非"写 1 清位"。
    //!   当前断言（返回结果、零睡眠、SIG_EN 状态）不受影响；
    //!   场景切换时由测试显式清零寄存器区。

    use alloc::boxed::Box;
    use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};

    use super::*;
    use crate::runtime::{SdhciDelay, set_delay};

    /// 泄漏一块寄存器缓冲区，仅以裸指针访问（避免与 `&mut` 别名）。
    /// 32 × u64 = 256 字节，8 字节对齐保证 u16/u32 访问满足对齐要求。
    fn fake_regs() -> usize {
        Box::leak(Box::new([0u64; 0x20])) as *mut [u64; 0x20] as usize
    }

    /// 重放模式：窗口内 XFER ISR（mask + notify 落空）。
    const MODE_XFER_ISR: u8 = 0;
    /// 重放模式：错误位锁存（错误不产生 ISR，SIG_EN 不变）。
    const MODE_ERROR_LATCH: u8 = 1;

    /// 确定性重放 Phase 2 窗口内的硬件/ISR 事件的 fake 延时提供者。
    ///
    /// `block_timeout_until` 被调用时，重放事件已经发生（对应真实时序中
    /// "unmask 之后、锁内条件检查之前"的窗口），随后按胶水层语义
    /// （关中断临界区内）检查条件：
    ///
    /// - MODE_XFER_ISR：硬件置位 XFER_COMPLETE sticky 位，ISR mask 信号
    ///   并 notify——此时任务尚未入队，notify 落空丢失。
    /// - MODE_ERROR_LATCH：硬件锁存错误位（NORM_ERROR + DAT_TIMEOUT），
    ///   错误不产生中断，无 ISR、无 mask。
    struct FakeIrqDelay {
        base: AtomicUsize,
        slept_ms: AtomicU64,
        mode: AtomicU8,
    }

    impl FakeIrqDelay {
        fn replay_event_in_window(&self) {
            let base = self.base.load(Ordering::Acquire);
            let sig_en_addr = base + SDHCI_NORM_INT_SIG_EN as usize;
            // 前置断言：unmask 必须先于阻塞发生（SIG_EN 已含 XFER 位），
            // 拦截"block 先于 unmask"的顺序回归。
            let sig_en = mmio_read::<u16>(sig_en_addr);
            assert!(
                sig_en & NORM_INT_XFER_COMPLETE != 0,
                "unmask_xfer_complete_signal 必须先于 block_timeout_until 执行"
            );

            let norm_addr = base + SDHCI_INT_STATUS_NORM as usize;
            let norm = mmio_read::<u16>(norm_addr);
            match self.mode.load(Ordering::Acquire) {
                MODE_ERROR_LATCH => {
                    // 硬件锁存错误位；错误不产生中断，SIG_EN 不变。
                    mmio_write::<u16>(norm_addr, norm | NORM_INT_ERROR);
                    let err_addr = base + SDHCI_INT_STATUS_ERR as usize;
                    let err = mmio_read::<u16>(err_addr);
                    mmio_write::<u16>(err_addr, err | ERR_INT_DAT_TIMEOUT);
                }
                _ => {
                    // 硬件在 unmask 后立即置位 XFER_COMPLETE sticky 位。
                    mmio_write::<u16>(norm_addr, norm | NORM_INT_XFER_COMPLETE);
                    // ISR mask XFER 信号（RMW 清除 XFER 位）。
                    mmio_write::<u16>(sig_en_addr, sig_en & !NORM_INT_XFER_COMPLETE);
                    // notify：队列为空，通知丢失（无动作）。
                }
            }
        }
    }

    impl SdhciDelay for FakeIrqDelay {
        fn delay_ms(&self, ms: u64) {
            self.slept_ms.fetch_add(ms, Ordering::Relaxed);
        }

        fn block_timeout_until(&self, timeout_ms: u64, condition: &dyn Fn() -> bool) -> bool {
            self.replay_event_in_window();
            if condition() {
                return false;
            }
            // 条件未观察到事件 → 协议失效路径：按真实语义睡满超时。
            self.delay_ms(timeout_ms);
            true
        }
    }

    /// Phase 2 IRQ 等待的两个确定性回归场景：
    ///
    /// - 场景 A：ISR 在 unmask 之后、锁内检查之前触发（notify 落空），
    ///   XFER_COMPLETE 等待必须即时返回，不得退化为睡满 10ms 超时兜底。
    /// - 场景 B：错误位在锁内检查前锁存时，条件立即返回错误路径，
    ///   且错误不产生 ISR。
    #[test]
    fn phase2_wait_lost_wakeup_and_error_latch_regressions() {
        static FAKE_DELAY: FakeIrqDelay = FakeIrqDelay {
            base: AtomicUsize::new(0),
            slept_ms: AtomicU64::new(0),
            mode: AtomicU8::new(MODE_XFER_ISR),
        };

        let base = fake_regs();
        FAKE_DELAY.base.store(base, Ordering::Release);
        FAKE_DELAY.slept_ms.store(0, Ordering::Relaxed);
        set_delay(&FAKE_DELAY);
        irq::irq_state_init(base);

        let sdhci = CviSdhci::new(base);

        // ── 场景 A：ISR 先于入队（notify 落空）──
        FAKE_DELAY.mode.store(MODE_XFER_ISR, Ordering::Release);
        let result = sdhci.poll_int_status(NORM_INT_XFER_COMPLETE);

        assert_eq!(result, Ok(()));
        // 关键断言：等待未落入 10ms 超时兜底——ISR 丢失的 notify 由
        // 锁内条件检查对 sticky 位的观察补偿，事件即时消费。
        assert_eq!(FAKE_DELAY.slept_ms.load(Ordering::Relaxed), 0);
        // 锁定等待后 SIG_EN 的 XFER 位保持 mask 稳态——配合 replay 入口
        // 断言（unmask 已置位）证明 unmask→ISR mask 序列完整执行。
        let sig_en = mmio_read::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize);
        assert_eq!(sig_en & NORM_INT_XFER_COMPLETE, 0);

        // ── 场景 B：错误位锁存（错误路径即时返回）──
        // 先锁定 STS_EN 门控：enable_interrupts_irq 后 STS_EN 必须含
        // NORM_INT_ERROR，否则错误场景在真实硬件上无意义（错误位不锁存）。
        sdhci.enable_interrupts_irq().unwrap();
        let sts_en = mmio_read::<u16>(base + SDHCI_NORM_INT_STS_EN as usize);
        assert_ne!(
            sts_en & NORM_INT_ERROR,
            0,
            "NORM_INT_ERROR 必须已加入 STS_EN 掩码，否则错误检测路径恒假"
        );
        // 清掉场景 A 的覆写残留（fake 不建模 W1C，见模块文档）。
        mmio_write::<u16>(base + SDHCI_INT_STATUS_NORM as usize, 0);
        FAKE_DELAY.mode.store(MODE_ERROR_LATCH, Ordering::Release);
        FAKE_DELAY.slept_ms.store(0, Ordering::Relaxed);

        let result = sdhci.poll_int_status(NORM_INT_XFER_COMPLETE);

        assert_eq!(result, Err(SdioError::Timeout));
        assert_eq!(FAKE_DELAY.slept_ms.load(Ordering::Relaxed), 0);
        // 错误不产生 ISR：SIG_EN 的 XFER 位未被 mask。
        let sig_en = mmio_read::<u16>(base + SDHCI_NORM_INT_SIG_EN as usize);
        assert_ne!(sig_en & NORM_INT_XFER_COMPLETE, 0);
    }
}
```

---

## 文件：`drivers/blk/sdhci-cv1800/src/regs.rs`

- 冲突面：路径冲突。dev 侧（自分叉点以来）对该文件的唯一改动是主线 #1951 的纯路径迁移（0 行内容变化）；分支侧的改动为内容变更。变基中 git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对内容一致。
- 变更前路径：`components/sdhci-cv1800/src/regs.rs` @ e04580c5c
- 变更后路径：`drivers/blk/sdhci-cv1800/src/regs.rs` @ HEAD

> 说明："变更前"为该位置在分叉基线（`e04580c5c`，即 dev 未动其内容时的版本）上的整块源码；"变更后"为变基后当前分支的整块源码。

整块数量：修改 2 / 新增 0 / 删除 4

#### `pub const NORM_INT_ENABLE_MASK: u16 = NORM_INT_CMD_COMPLETE`（修改）
变更前（第 56–62 行）：
```rust
/// 组合掩码
/// Status Enable: 使能所有需要的状态位
pub const NORM_INT_ENABLE_MASK: u16 = NORM_INT_CMD_COMPLETE
    | NORM_INT_XFER_COMPLETE
    | NORM_INT_BUF_WR_READY
    | NORM_INT_BUF_RD_READY
    | NORM_INT_CARD_INT;
```

变更后（第 56–67 行）：
```rust
/// 组合掩码
/// Status Enable: 使能所有需要的状态位。
/// 含 NORM_INT_ERROR：按 SDHCI 规范，INT_STATUS 位仅在对应 STS_EN 置位时
/// 锁存——缺 bit15 时错误检测路径（poll_status_once 错误分支与 Phase 2
/// 条件中的错误项）恒假。错误位不进入 SIG_EN（见 NORM_INT_SIG_MASK），
/// 不产生中断，仅由轮询/条件检查消费。
pub const NORM_INT_ENABLE_MASK: u16 = NORM_INT_CMD_COMPLETE
    | NORM_INT_XFER_COMPLETE
    | NORM_INT_BUF_WR_READY
    | NORM_INT_BUF_RD_READY
    | NORM_INT_CARD_INT
    | NORM_INT_ERROR;
```

#### `pub const NORM_INT_SIG_MASK: u16 = NORM_INT_CARD_INT;`（修改）
变更前（第 77–78 行）：
```rust
/// Signal Enable: 仅使能 CARD_INT (PIO 事件由 wait 函数直接轮询 INT_STATUS)
pub const NORM_INT_SIG_MASK: u16 = NORM_INT_CARD_INT;
```

变更后（第 82–83 行）：
```rust
/// 信号使能：仅使能 CARD_INT（XFER_COMPLETE 信号由 poll_int_status 阻塞前动态 un-mask）
pub const NORM_INT_SIG_MASK: u16 = NORM_INT_CARD_INT;
```

#### `pub const CMD5_READY_TIMEOUT: u32 = 1_000;`（删除）
变更前（第 180–180 行）：
```rust
pub const CMD5_READY_TIMEOUT: u32 = 1_000;
```
变更后：无（当前分支中该整块已不存在）

#### `pub const PIO_TIMEOUT: u32 = 1_000_000;`（删除）
变更前（第 182–182 行）：
```rust
pub const PIO_TIMEOUT: u32 = 1_000_000;
```
变更后：无（当前分支中该整块已不存在）

#### `pub const FUNC_READY_TIMEOUT: u32 = 1_000;`（删除）
变更前（第 183–183 行）：
```rust
pub const FUNC_READY_TIMEOUT: u32 = 1_000;
```
变更后：无（当前分支中该整块已不存在）

#### `pub const FUNC_READY_DELAY: u32 = 100_000;`（删除）
变更前（第 184–184 行）：
```rust
pub const FUNC_READY_DELAY: u32 = 100_000;
```
变更后：无（当前分支中该整块已不存在）

---

## 文件：`drivers/blk/sdhci-cv1800/src/runtime.rs`

- 冲突面：路径冲突。dev 侧（自分叉点以来）对该文件的唯一改动是主线 #1951 的纯路径迁移（0 行内容变化）；分支侧的改动为内容变更。变基中 git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对内容一致。
- 变更前路径：`components/sdhci-cv1800/src/runtime.rs` @ e04580c5c
- 变更后路径：`drivers/blk/sdhci-cv1800/src/runtime.rs` @ HEAD

> 说明："变更前"为该位置在分叉基线（`e04580c5c`，即 dev 未动其内容时的版本）上的整块源码；"变更后"为变基后当前分支的整块源码。

整块数量：修改 3 / 新增 0 / 删除 0

#### `//! 文件头注释`（修改）
变更前（第 1–6 行）：
```rust
//! OS timing capability injection.
//!
//! The controller driver needs millisecond delays and a CPU-yield while polling
//! hardware, but must not bind to any specific kernel's task runtime. The OS
//! glue installs a [`SdhciDelay`] provider once via [`set_delay`]; the driver
//! reaches it through [`delay`].
```

变更后（第 1–6 行）：
```rust
//! OS 时序能力注入。
//!
//! 控制器驱动需要毫秒级延迟和中断驱动的阻塞等待，
//! 但不能绑定到任何特定内核的任务运行时。
//! OS 胶水层通过 [`set_delay`] 一次性安装 [`SdhciDelay`] 提供者；
//! 驱动通过 [`delay`] 访问它。
```

#### `pub trait SdhciDelay: Send + Sync + 'static {`（修改）
变更前（第 10–16 行）：
```rust
/// Timing capabilities the SDHCI controller needs from the OS.
pub trait SdhciDelay: Send + Sync + 'static {
    /// Blocking delay for the given milliseconds.
    fn delay_ms(&self, ms: u64);
    /// Yield the CPU to other tasks while polling hardware.
    fn yield_now(&self);
}
```

变更后（第 10–47 行）：
```rust
/// SDHCI 控制器所需的 OS 时序能力。
pub trait SdhciDelay: Send + Sync + 'static {
    /// 阻塞延迟指定毫秒数。
    fn delay_ms(&self, ms: u64);
    /// 阻塞当前任务直至 `condition` 满足或超时。
    /// 条件满足返回 `false`，超时返回 `true`。
    ///
    /// # 丢唤醒协议（实现必须满足）
    ///
    /// `condition` 的最终检查与任务入队必须发生在同一关中断临界区内，
    /// 使“检查后、入队前 ISR 已发布完成事件”的情况不可能出现：
    ///
    /// - ISR 在锁内检查前触发：其 latch 的硬件状态位（XFER_COMPLETE sticky）
    ///   由 `condition` 观察到，任务不进入睡眠直接返回；
    /// - ISR 在入队后触发：notify 命中已入队的任务。
    ///
    /// 调用方在调用前负责重新打开中断信号（unmask）；开信号与入队之间的
    /// ISR 不会丢事件——它 latch 的硬件状态位由锁内 `condition` 检查看到。
    ///
    /// 本方法在中断开启的 task 上下文被调用（禁止在 ISR 上下文调用）；
    /// 上文的关中断临界区由实现自行建立（如 `SpinNoIrq` 锁），
    /// 调用方不负责关中断。
    ///
    /// # 单 waiter 契约
    ///
    /// 调用方保证至多一个任务同时阻塞于此方法（由 SDIO 总线锁序列化）。
    /// OS 胶水层可依赖此保证使用单一共享唤醒队列。
    ///
    /// 默认实现退化为“检查一次 + 睡满超时”，事件即时性由调用方重检兜底，
    /// 兼容未实现条件等待的 OS 胶水层。
    fn block_timeout_until(&self, timeout_ms: u64, condition: &dyn Fn() -> bool) -> bool {
        if condition() {
            return false;
        }
        self.delay_ms(timeout_ms);
        true
    }
}
```

#### `pub fn set_delay(provider: &'static dyn SdhciDelay) {`（修改）
变更前（第 20–29 行）：
```rust
/// Installs the timing capability provider. Call once during init, before
/// driving the controller.
pub fn set_delay(provider: &'static dyn SdhciDelay) {
    let boxed = alloc::boxed::Box::new(provider);
    let ptr = alloc::boxed::Box::into_raw(boxed);
    let old = DELAY.swap(ptr, Ordering::AcqRel);
    if !old.is_null() {
        unsafe { drop(alloc::boxed::Box::from_raw(old)) };
    }
}
```

变更后（第 51–60 行）：
```rust
/// 安装时序能力提供者。在初始化期间、操作控制器前调用一次。
/// 不得与 [`delay`] 并发调用。
pub fn set_delay(provider: &'static dyn SdhciDelay) {
    let boxed = alloc::boxed::Box::new(provider);
    let ptr = alloc::boxed::Box::into_raw(boxed);
    let old = DELAY.swap(ptr, Ordering::AcqRel);
    if !old.is_null() {
        unsafe { drop(alloc::boxed::Box::from_raw(old)) };
    }
}
```

---

## 文件：`drivers/net/aic8800/src/fdrv/thread/rx.rs`

- 冲突面：路径冲突。dev 侧（自分叉点以来）对该文件的唯一改动是主线 #1951 的纯路径迁移（0 行内容变化）；分支侧的改动为内容变更。变基中 git 以重命名检测将分支补丁原样带到新路径，逐 hunk 比对内容一致。
- 变更前路径：`components/aic8800/src/fdrv/thread/rx.rs` @ e04580c5c
- 变更后路径：`drivers/net/aic8800/src/fdrv/thread/rx.rs` @ HEAD

> 说明："变更前"为该位置在分叉基线（`e04580c5c`，即 dev 未动其内容时的版本）上的整块源码；"变更后"为变基后当前分支的整块源码。

整块数量：修改 1 / 新增 0 / 删除 0

#### `pub fn start(bus: Arc<WifiBus>) {`（修改）
变更前（第 60–107 行）：
```rust
/// 启动 wifi-rx 线程
pub fn start(bus: Arc<WifiBus>) {
    log::debug!("[wifi-rx] thread starting");
    // RX poll kicker 仅 DC/DW 启用(与 TX kicker 对称)。D80/8801 走 upstream/dev
    // 的纯事件(ISR)驱动路径,不启动周期 kicker。
    if bus.transport.is_dual_pipe() {
        start_rx_poll_kicker(bus.clone());
    }
    crate::runtime::runtime().spawn_poll_task(
        "wifi-rx",
        alloc::boxed::Box::new(move |cx| {
            // 检查总线状态
            if *bus.state.lock() == BusState::Down {
                return Poll::Ready(());
            }

            // 检查并清除 ISR 标志
            if bus.rx.irq_pending.swap(false, Ordering::AcqRel) {
                RX_WAKE_COUNT.fetch_add(1, Ordering::Relaxed);
            }

            // 处理所有待读数据（内部会 mask CARD_INT，但不 unmask）
            process_rx_frames(&bus);

            // 先注册 waker，再 unmask CARD_INT
            // 这样 ISR 触发时 waker 已经就位，不会丢失唤醒
            bus.rx.irq_waker.register(cx.waker());

            // 关键：先 register waker，再 unmask CARD_INT
            // 如果 ISR 在 unmask 后立即触发，waker 已经注册好了
            bus.transport.unmask_card_irq();

            // 若本批有数据帧入队,驱动网络栈处理(AP/STA 收包)。
            invoke_rx_data_callback();

            // 双重检查：如果 ISR 在 register 和 unmask 之间触发了
            if bus.rx.irq_pending.swap(false, Ordering::AcqRel) {
                process_rx_frames(&bus);
                bus.transport.unmask_card_irq();
                invoke_rx_data_callback();
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }

            Poll::Pending
        }),
    );
}
```

变更后（第 60–106 行）：
```rust
/// 启动 wifi-rx 线程
pub fn start(bus: Arc<WifiBus>) {
    log::debug!("[wifi-rx] thread starting");
    // RX poll kicker: D80/8801 的 ISR 驱动路径不可靠（PLIC IRQ 在 probe 阶段
    // 未使能，固件响应在 TX->RX 的单次 wake 之后才到达时 RX 线程无人唤醒），
    // 用 10ms kicker 兜底保证轮询响应和异步入站帧不丢失。
    start_rx_poll_kicker(bus.clone());
    crate::runtime::runtime().spawn_poll_task(
        "wifi-rx",
        alloc::boxed::Box::new(move |cx| {
            // 检查总线状态
            if *bus.state.lock() == BusState::Down {
                return Poll::Ready(());
            }

            // 检查并清除 ISR 标志
            if bus.rx.irq_pending.swap(false, Ordering::AcqRel) {
                RX_WAKE_COUNT.fetch_add(1, Ordering::Relaxed);
            }

            // 处理所有待读数据（内部会 mask CARD_INT，但不 unmask）
            process_rx_frames(&bus);

            // 先注册 waker，再 unmask CARD_INT
            // 这样 ISR 触发时 waker 已经就位，不会丢失唤醒
            bus.rx.irq_waker.register(cx.waker());

            // 关键：先 register waker，再 unmask CARD_INT
            // 如果 ISR 在 unmask 后立即触发，waker 已经注册好了
            bus.transport.unmask_card_irq();

            // 若本批有数据帧入队,驱动网络栈处理(AP/STA 收包)。
            invoke_rx_data_callback();

            // 双重检查：如果 ISR 在 register 和 unmask 之间触发了
            if bus.rx.irq_pending.swap(false, Ordering::AcqRel) {
                process_rx_frames(&bus);
                bus.transport.unmask_card_irq();
                invoke_rx_data_callback();
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }

            Poll::Pending
        }),
    );
}
```

---
