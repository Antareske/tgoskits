# StarryOS 代码整理优化计划（第二版）

> 生成日期：2026-07-03
> 适用范围：`os/StarryOS/kernel`、`components/axcpu`、`scripts/axbuild`
> 背景：近期 PR 数量激增导致代码质量下降。本报告在对上一版调查（`starry-code-quality-report.md`）逐条核实、修正失真项之后，给出可直接执行的整理清单。
>
> 本版原则：**只列已核实到具体文件/行/结构的真实问题，给出可操作的重构边界**，不纠结行数是否精确。每一项都标注了核实状态与落地障碍。

---

## 0. 与上一版报告的差异说明

上一版报告方向正确，多数定位（文件/行/结构）准确，但存在以下已核实的失真，本版已修正：

| 项目 | 上一版说法 | 核实结论 |
|------|-----------|---------|
| kprobe.rs panic 清单 | 列 ~17 处 `.unwrap()` | **失真**。该文件仅 2 处 `unwrap`（`:218` `pop().unwrap()`、`:248` `try_lock().unwrap()`），其余所列行号实为 uprobe 的 `.expect()`。清单陈旧，动手前须重新审计。 |
| Ptrace FP "几乎相同的副本" | 5 份几乎相同 | **措辞不准**。5 份 `save/restore` 各 arch 用不同 FP 类型/字段，并非机械副本；真正的冗余是 `PtraceStopFpData` 重造了 axcpu 已有类型（见 §1.1）。 |
| singlestep "4 份 / 3 份" | setup 4 份、restore 3 份 | **修正**。`setup` 为 **3 份同构（riscv/aarch64/loongarch64）+ x86_64 特例**；`restore` 中 aarch64/loongarch64 已合并，实为 **2 份**。 |
| pseudofs / axbuild 规模 | pseudofs ~20 文件、axbuild ~220 文件/~22300 行 | **偏低**。实测 pseudofs 54 文件、axbuild 236 文件 / 53919 行。 |
| someboot allow(dead_code) 路径 | `components/someboot` | **路径错**。实际在 `platforms/someboot`，约 35 处。 |

准确项（可直接引用）：`unwrap`/`expect` 计数、依赖冲突分析（含 tock-registers 0.9/0.10）、结构体 body 行数、board.rs/StageLog/Snapshot 重复、glob re-export、`ARCH_SPECS` 已存在、八进制权限 50+。

---

## 1. 诉求一：arch 条件编译下沉到 axcpu

目标：把散落在 kernel 里、按 `#[cfg(target_arch = ...)]` 展开的通用 CPU 能力，收敛到 `components/axcpu` 的各 arch 模块，通过 `ax_cpu::*` 统一导出。

### 1.1 【P0】消除 `PtraceStopFpData`，直接复用 axcpu FP 类型

**位置**：`os/StarryOS/kernel/src/task/mod.rs`
- 结构定义：`:773-810`（5 个 cfg 版本的 `PtraceStopFpData`）
- 保存函数：`save_current_fp_for_ptrace` `:1619-1682`（5 个 cfg 版本）
- 恢复函数：`restore_current_fp_for_ptrace` `:1684-1752`（5 个 cfg 版本）

**核实到的问题本质**：`PtraceStopFpData` 只是把 `ax_cpu::FpState`/`FpuState`/`FxsaveArea` 的字段**逐一拆出来再原样装回去**。例如 aarch64：

```rust
// save: 把 FpState 拆进 PtraceStopFpData
let mut fp = ax_cpu::FpState::default();
fp.save();
PtraceStopFpData { regs: fp.regs, fpcr: fp.fpcr, fpsr: fp.fpsr }
// restore: 再从 PtraceStopFpData 装回 FpState
let fp_state = ax_cpu::FpState { regs: fp.regs, fpcr: fp.fpcr, fpsr: fp.fpsr };
fp_state.restore();
```

axcpu 侧的 `FpState`（`components/axcpu/src/{riscv,aarch64}/context.rs`）、`FpuState`（loongarch64）、`FxsaveArea`（x86_64）**均已 `#[derive(Clone, Copy)]` 且自带 `save()`/`restore()`**，并通过 `ax_cpu::*` 扁平导出。

**落地方案**：
1. 让 `ptrace_stop_fp_data` 直接存 arch 原生 FP 类型（对 x86_64 已经是 `FxsaveArea`，只是套了一层 newtype）。定义一个薄的 kernel 侧别名 `type PtraceFpSnapshot = ax_cpu::<arch FP type>` 即可，无需字段拆装。
2. 删除 5 份 `PtraceStopFpData` 定义与 5×2 份 cfg 函数体，`save/restore` 缩减为「调用 axcpu 的 save/restore + 存/取快照」。

**唯一障碍（必须处理）**：riscv 的 `fs` 字段。
- 保存时 kernel 额外执行 `fp.fs = sstatus::read().fs()`（`:1623`），而 `PtraceStopFpData` **丢弃了** `fs`。
- 恢复时 kernel 硬编码 `FS::Dirty`（`:1693,1697,1700`）而非用保存值。

建议：在 axcpu 的 riscv `FpState::save()` 内部就填好 `fs`（把 `sstatus` 读取下沉到 axcpu），恢复策略（保留原值 vs 强制 Dirty）作为 axcpu 上的明确方法暴露，kernel 不再直接碰 `riscv::register::sstatus`。这样 kernel 侧 riscv 分支也能与其他 arch 对齐。

**收益**：删除约 5 结构 + 10 函数的 cfg 样板；ptrace FP 语义集中到 axcpu，避免今后新增 arch 时再抄一遍。

**回归保护**：改动前先加确定性测试——对每个 arch 构造已知 FP 内容，`save` 后改寄存器再 `restore`，断言恢复一致；riscv 额外断言 `fs` 语义。

### 1.2 【P1】`dump_user_crash_context` 下沉为 axcpu 上的寄存器格式化

**位置**：`os/StarryOS/kernel/src/task/signal.rs:96-171`（4 个 arch cfg 块 + 1 个 fallback）。

**核实修正**：各 arch 打印的**寄存器数量差异很大**——riscv 打印全部 31 个 GPR，aarch64/x86_64/loongarch64 只打印少数关键寄存器。这不是纯机械重复，是有意的详略取舍。

**落地方案**：在 axcpu 的 `UserContext`（各 arch 已有该类型）上实现 `fmt::Display` 或 `format_registers()`，把「哪些寄存器、什么布局」作为各 arch 的实现细节下沉到 axcpu。kernel 侧 `dump_user_crash_context` 收敛为一行调用 + `dump_user_backtrace`。**下沉时须保留各 arch 现有详略度**，不要统一成同一套字段。

### 1.3 【P1】ptrace 单步（singlestep）下沉

**位置**：`os/StarryOS/kernel/src/syscall/task/ptrace.rs`（`:1156` 起）。

**核实修正**：
- `ptrace_setup_singlestep`：**3 份同构（riscv/aarch64/loongarch64）+ x86_64 特例**。三份同构差异仅在：断点指令常量、指令宽度、icache flush 方式、`next_pc` 计算；x86_64 只设 `RFLAGS.TF` 位，不参与这三份重复。
- `ptrace_restore_singlestep_insn`：aarch64/loongarch64 已用 `cfg(any(...))` 合并，实为 **2 份**（非上一版所说 3 份）。

**落地方案**：把「写断点指令 / 恢复原指令 / flush icache / 计算下一 PC」定义为 axcpu 各 arch 的软件单步原语（如 `SoftwareStep` 能力）。x86_64 用硬件 TF 位实现同一 trait。kernel 只依赖统一接口，三份同构 + 1 特例收敛为一处调用点。此项依赖 axcpu 新增接口，改动面较大，排在 1.1/1.2 之后。

### 1.4 arch 条件编译整体巡检建议

上一版对 `RiscvUserRegs`/`Aarch64UserRegs`/`LoongarchUserRegs`/`X8664UserRegs`（`ptrace.rs:95-201`）的定位准确。这些 wire-format 寄存器结构属于 ptrace ABI，**是否下沉需谨慎**——它们是面向用户态的固定布局，与 axcpu 内部 `UserContext` 不必强行统一。建议保留在 kernel，仅把「`UserContext` ↔ UserRegs 的转换」按 arch 收敛到一处 trait 实现，避免 syscall 层散落转换逻辑。

---

## 2. 诉求二：大文件 / 大结构体职责拆分

### 2.1 超大文件清单（已核实行数）

| 文件 | 行数 | 拆分建议 |
|------|-----|---------|
| `syscall/task/ptrace.rs` | 2423 | **P0**。按职责拆：`attach/detach`、`寄存器读写(UserRegs 转换)`、`singlestep`、`syscall trace`、`stop/resume 事件`。目前所有 ptrace 逻辑挤在一个文件。 |
| `task/mod.rs` | 1942 | **P0**。见 §2.2，`ProcessData`/`Thread` 为 God Object。 |
| `pseudofs/proc.rs` | 1805 | **P1**。按 `/proc` 下的节点族拆（`self/`、`pid/`、全局节点等）。 |
| `pseudofs/dev/card0.rs` | 1779 | **P1**。DRM/GPU 节点，可按 ioctl 族或功能子系统拆。 |
| `pseudofs/usbfs/mod.rs` | 1732 | **P1**。与同目录 `manager.rs`(1391) 一并审视职责边界。 |
| `syscall/fs/aio.rs` | 1499 | **P2**。异步 IO，按 submit/poll/cancel 拆。 |
| `pseudofs/usbfs/manager.rs` | 1391 | **P2**。 |
| `perf/hw.rs` | 1196 | **P2**。 |
| `pseudofs/sysfs.rs` | 1185 | **P2**。 |
| `syscall/mm/mmap.rs` | 1123 | **P2**。 |

> 完整 >1000 行文件共 13 个（含上表），另有 `perf/task.rs`(1079)、`syscall/fs/io.rs`(1011)、`pseudofs/dev/tty/terminal/ldisc.rs`(1011)、`syscall/sys.rs`(1009)。

### 2.2 【P0】`ProcessData` God Object 拆分

**位置**：`os/StarryOS/kernel/src/task/mod.rs`，`ProcessData` 定义约 `:611` 起，`new()` 构造见 `:812-895`。

**核实到的问题**：单个结构体聚合了互不相关的子系统状态（从 `new()` 字段可见）：
- 地址空间 / 堆：`aspace`、`heap_top`、`vm_aspace_shared`、`aspace_slot_released`
- 信号：`signal`、`signal_actions`
- ptrace：`ptrace_*`（约 14 个字段，`:862-876`）
- 命名空间 / futex / 定时器：`nsproxy`、`futex_table`、`posix_timers`
- 作业控制：`job_control`、`cont_event`、`exit_signal`
- 杂项策略位：`umask`、`nice`、`membarrier_state`、`dumpable`、`thp_disable`、`personality`

**落地方案**：按「变更原因 / 拥有的不变量」拆出子结构，例如 `PtraceState`（14 个 ptrace 字段单独成组，独立锁）、`ProcessMemory`（aspace/heap/slot）、`ProcessPolicy`（umask/nice/personality 等原子位）。`ProcessData` 保留为组合根。ptrace 字段单拆收益最大——它们生命周期一致、且 §1 的 FP/singlestep 重构也会集中触碰这块。

**注意**：`ProcessData::new()` 中已有关于锁顺序的注释（`:888-892`：临时 `SpinNoIrq` guard 与 `Mutex<AddrSpace>` 的嵌套问题）。拆分时须保持该锁序不变，并在移动字段时重新审视 guard 生命周期。

### 2.3 【P1】`Thread` 结构

**位置**：`task/mod.rs`，`Thread` 定义约 `:97` 起。上一版 body 行数定位准确。相较 `ProcessData` 拆分紧迫性低，可在 `ProcessData` 拆分后顺带评估。

### 2.4 拆分执行纪律

- 每个文件拆分都是**纯搬移 + 重导出**，不改行为；先拆文件、跑 `cargo xtask clippy --package <crate>` + 相关测试，再考虑结构调整。
- `ProcessData` 子结构拆分属于有行为风险的改动，须先有覆盖 ptrace / signal / mm 路径的回归，逐子系统小步提交。

---

## 3. 诉求三：axbuild 重复逻辑清理

`scripts/axbuild` 实测 236 文件 / 53919 行。以下为已核实的重复。

### 3.1 【P0】三份 board 发现逻辑

**位置**：
- `scripts/axbuild/src/arceos/board.rs`（178 行）
- `scripts/axbuild/src/starry/board.rs`（223 行）
- `scripts/axbuild/src/axvisor/board.rs`（171 行）

三者都有同名同构函数：`board_dir`、`load_board_file`、`board_default_list`、`find_board`、`board_names`。

**核实到的真实差异（设计抽象时必须处理）**：
1. **Board 字段集不同**：arceos 为 `package/target/build_config`；starry 为 `target/build_info`；axvisor 的 `Board`（`axvisor/board.rs:12`）另有一套。
2. **错误处理策略不同**：starry 的 `board_default_list` 用 `let Ok(...) else continue` 容错跳过坏条目；arceos 用 `?` 严格失败。
3. 错误消息里的 OS 名不同。

**落地方案**：抽 `trait BoardDiscovery`（关联类型 `Board`、`BoardFile`），把 `board_dir/find_board/board_names/board_default_list` 的**遍历骨架**放进 trait 默认方法或一个泛型 helper；各 OS 只实现「目录名、如何解析单个 board 文件、遇到坏条目是跳过还是报错」。不要强行统一 `Board` 字段——用关联类型保留差异。

### 3.2 【P1】`StageLog` 重复定义

**位置**：定义了两次——`scripts/axbuild/src/context/mod.rs:450` 与 `scripts/axbuild/src/starry/build.rs:627`。

**落地方案**：以 `context/mod.rs` 版本为准（更靠近公共上下文层），删除 `starry/build.rs` 副本并改为引用。核实两处字段/方法是否完全一致；若 starry 版本有额外字段，合并进公共版本或用组合。

### 3.3 【P1】`load_uboot_config` / `load_board_config` 三份

三个 OS 各一份 uboot/board 配置加载。与 §3.1 同源，建议在同一次重构里一并收敛到 `BoardDiscovery` 或相邻的配置加载 helper。

### 3.4 【P1】用 `ARCH_SPECS` 消除 arch 字符串匹配链

**现状**：`target.starts_with("x86_64"/"aarch64"/"riscv"/"loongarch")` 匹配链散落 **30 处 / 6 文件**：
- `build/platform.rs`、`build/std_build.rs`、`clippy/targets.rs`、`backtrace/paths.rs`（**重构价值集中在这几个**）
- `arceos/build/tests.rs`、`build/tests/target_specs.rs`（测试文件，价值低）

**现有基础设施**：`scripts/axbuild/src/context/arch.rs` 已有成熟的中心表 `ARCH_SPECS`（字段 `arch/target/default_rootfs_image/cross_compile`，`cross_compile` 内含 `llvm_target/cmake_system_processor/guest_tool_dir/gnu_tool_prefix/qemu_user_binaries`）及查表函数 `arch_spec_for_target()`。

**落地方案**：把 `platform.rs`/`std_build.rs`/`clippy/targets.rs`/`backtrace/paths.rs` 里的 `starts_with` 分支改为查 `ARCH_SPECS`；若需要新属性（如各处分支产出的某个字符串），**扩展 `ArchSpec` 字段**而非新增 if/else。这完全契合该表已有的设计意图，无需引入新抽象。测试文件里的匹配可保留或按需迁移。

### 3.5 【P2】Snapshot 结构体族

上一版定位准确：`scripts/axbuild/src/context/types.rs:49-147` 有约 9 组 Qemu/Uboot Snapshot 结构。属于数据结构层重复，优先级低于骨架逻辑重复；在 §3.1 完成后评估是否值得用泛型/宏收敛。

---

## 4. 附带质量项（非三诉求，但本轮可顺手处理）

这些是巡检中确认存在、但不属于三诉求核心的项，按低优先级列出，供整理时机动处理：

- **`unwrap`/`expect` 审计**：kernel 全量 `unwrap`=66、`expect`=67（已核实）。**先重新审计 `kprobe.rs`**（上一版清单陈旧，真实仅 `:218`/`:248` 两处 `unwrap`）。`mm/loader.rs` 的 `try_into().unwrap()` 群（`:334-467`）与 `ipc/shm.rs` 的 panic 定位准确，可评估是否改 `Result`。
- **TODO/FIXME/HACK**：全仓约 53 处；`net/opt.rs` 集中 18 处 TODO。属技术债登记，非本轮重构目标。
- **依赖冲突**：`tock-registers` 0.9 与 0.10 并存（代码内已有注释确认）。如本轮触及相关 crate 可顺带统一。
- **`allow(dead_code)`**：kernel 9 处、`platforms/someboot`（**注意路径**）约 35 处。属清理项，非重构。

---

## 5. 建议执行顺序

1. **先修工具认知**：重新审计 `kprobe.rs` panic 清单，纠正上一版陈旧数据（§0 / §4）。
2. **P0 低风险纯搬移**：拆 `ptrace.rs`(2423) 与 `task/mod.rs`(1942) 为多文件（§2.1），只搬不改，跑 clippy + 测试。
3. **P0 axcpu 下沉**：`PtraceStopFpData` 消除（§1.1），先加各 arch FP save/restore 回归测试，处理 riscv `fs` 障碍。
4. **P0 axbuild 骨架**：`BoardDiscovery` trait 收敛三份 board 逻辑（§3.1），连带 `StageLog`(§3.2)、uboot/board config(§3.3)。
5. **P1**：`dump_user_crash_context` 下沉（§1.2）、`ARCH_SPECS` 消除匹配链（§3.4）、`ProcessData` 子结构拆分（§2.2）。
6. **P1/P2**：singlestep 下沉（§1.3，依赖 axcpu 新接口）、其余大文件拆分、Snapshot 收敛。

**通用纪律**（遵循 AGENTS.md）：改逻辑先加会在旧实现下失败的确定性回归测试 → 验证失败 → 改 → 验证通过；每改一个 crate 跑 `cargo xtask clippy --package <crate>`；改完 `cargo fmt`；不用 `allow` 掩盖 clippy；小步提交、行为稳定优先。
