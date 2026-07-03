# StarryOS 代码质量调查报告

> 日期: 2026-07-02  
> 范围: `os/StarryOS/`, `components/axcpu/`, `components/starry-*`, `scripts/axbuild/`  
> 目标: 为新一轮 Starry 代码整理优化提供数据支持

---

## 目录

1. [总览与关键数据](#1-总览与关键数据)
2. [Arch 条件编译：可下沉到 axcpu 的模式](#2-arch-条件编译可下沉到-axcpu-的模式)
3. [大文件与大型结构体拆分建议](#3-大文件与大型结构体拆分建议)
4. [axbuild 重复逻辑清理与复用](#4-axbuild-重复逻辑清理与复用)
5. [错误处理与 panic 路径审计](#5-错误处理与-panic-路径审计)
6. [模块边界与可见性](#6-模块边界与可见性)
7. [依赖分析](#7-依赖分析)
8. [TODO/FIXME/HACK 积压](#8-todofixmehack-积压)
9. [死代码与未使用导入](#9-死代码与未使用导入)
10. [测试覆盖率缺口](#10-测试覆盖率缺口)
11. [大 match 语句与字符串化模式](#11-大-match-语句与字符串化模式)
12. [行动建议优先级矩阵](#12-行动建议优先级矩阵)

---

## 1. 总览与关键数据

### 1.1 涉及范围

| 层级 | 模块 | 规模 |
|------|------|------|
| 内核核心 | `os/StarryOS/kernel/src/` | **189 源文件** |
| 组件 | `components/starry-process/`, `starry-signal/`, `starry-vm/`, `axcpu/`, `axerrno/` | 5 个 crate |
| 构建工具 | `scripts/axbuild/src/` | **~220 源文件, ~22,300 行** |
| 伪文件系统 | `os/StarryOS/kernel/src/pseudofs/` | ~20 源文件 |

### 1.2 核心统计

| 指标 | 数值 |
|------|------|
| >500 行的源文件 | **52 个** |
| >1,000 行的源文件 | **14 个** |
| >2,000 行的源文件 | **1 个** (`ptrace.rs`, 2,423 行) |
| body >100 行的 struct 定义 | **3 个** |
| 非测试 `unwrap()` 总计 | **66 处** (kernel) |
| 非测试 `expect()` 总计 | **67 处** (kernel) |
| TODO/FIXME/HACK 总计 | **54 处** |
| 带测试的文件比例 | **5.8%** (11/189) |
| axbuild 中的重度重复模块对 | **11 组** |

---

## 2. Arch 条件编译：可下沉到 axcpu 的模式

axcpu 已经提供了清晰的 per-arch 抽象层（`x86_64/`, `aarch64/`, `riscv/`, `loongarch64/`，每个含 `context.rs`, `trap.rs`, `uspace.rs` 等），但 StarryOS 内核代码**绕过了**这个抽象，直接在 kernel 代码中写 `#[cfg(target_arch = "...")]` 门控的代码块。这是最核心的质量问题。

### 2.1 HIGH: Ptrace FP 保存/恢复 — 4 份几乎相同的副本

**位置:** `os/StarryOS/kernel/src/task/mod.rs`

| 函数 | 行号 | 架构 | 重复度 |
|------|------|------|--------|
| `save_current_fp_for_ptrace()` | 1619-1682 | riscv64 / aarch64 / loongarch64 / x86_64 / fallback | 5 份, ~60行/份 |
| `restore_current_fp_for_ptrace()` | 1684-1752 | riscv64 / aarch64 / loongarch64 / x86_64 / fallback | 5 份, ~60行/份 |
| `PtraceStopFpData` struct | 773-810 | riscv64 / aarch64 / loongarch64 / x86_64 / fallback | 5 份 |

**模式描述:** 每个 arch 遵循相同的 3 步流程：`save FPU → extract fields → insert into HashMap`。仅 FP 字段名和类型不同。

**建议:** 在 axcpu 中新增 `PtraceFpSaveRestore` trait，为每个 arch 提供 `fn save_fp_for_ptrace() -> PtraceStopFpData` 和 `fn restore_fp_for_ptrace(data: &PtraceStopFpData)`。kernel 侧只需调用 trait 方法。

### 2.2 HIGH: `dump_user_crash_context` 寄存器打印 — 4 份副本

**位置:** `os/StarryOS/kernel/src/task/signal.rs:97-171`

每个 arch 的代码块结构相同：`warn!("user register dump:\n pc: {:#x}, ra: {:#x}, sp: {:#x}, ...")`，只是寄存器名不同（pc/sepc/era/rip, ra/x30/lr）。

**建议:** 在 axcpu 的 `UserContext` 上新增 `fn format_registers(&self, writer: &mut impl fmt::Write)` 方法。kernel 侧变为 `warn!("user register dump:\n{}", uctx.register_dump())`。

### 2.3 HIGH: ptrace 单步设置 — 3 份结构相同的实现

**位置:** `os/StarryOS/kernel/src/syscall/task/ptrace.rs`

| 函数 | 行号 | 架构 | 代码量 |
|------|------|------|--------|
| `ptrace_setup_singlestep()` | 1156-1320 | x86_64 / riscv64 / aarch64 / loongarch64 | 4 份, 40-60行/份 |
| `ptrace_restore_singlestep_insn()` | 1322-1350+ | riscv64 / aarch64 / loongarch64 | 3 份, 10-20行/份 |

riscv64/aarch64/loongarch64 的实现**结构完全相同**：读指令 → 计算下一条 PC → 写断点指令 → 保存原始指令。仅 3 个参数不同：断点指令常量、指令宽度、flush 方式。

**建议:** 在 axcpu 中定义 `SinglestepBackend` trait：
```rust
trait SinglestepBackend {
    const BREAK_INSN: Insn;           // 断点指令常量
    type Insn;                         // u16 / u32
    fn compute_next_pc(pc: usize, insn: Self::Insn) -> usize;
    fn flush_icache(addr: usize, len: usize);
    fn write_insn_as_breakpoint(addr: usize) -> Result<Self::Insn>;
    fn restore_original_insn(addr: usize, original: Self::Insn);
}
```
kernel 侧 `ptrace_setup_singlestep` 和 `ptrace_restore_singlestep_insn` 变为 trait 上的一个泛型实现。

### 2.4 MEDIUM: Ptrace 寄存器类型定义散落

**位置:** `os/StarryOS/kernel/src/syscall/task/ptrace.rs:87-200+`

4 个 arch 各自定义了 `RiscvUserRegs`, `Aarch64UserRegs`, `LoongarchUserRegs`, `X8664UserRegs`（以及对应的 `*FpRegs`），然后通过 type alias 做 cfg dispatch。

**建议:** 将 per-arch 的用户寄存器类型定义移到 `axcpu::ptrace::` 模块中，与 `UserContext` 的定义放在一起（这些寄存器类型本质上是从 `UserContext` 转换而来的）。

### 2.5 MEDIUM: Legacy x86_64 syscall wrapper — 25+ 处

**位置:** `os/StarryOS/kernel/src/syscall/fs/ctl.rs` 和 `os/StarryOS/kernel/src/syscall/mod.rs`

模式重复 25 次以上：
```rust
#[cfg(target_arch = "x86_64")]
pub fn sys_link(old_path: ..., new_path: ...) -> AxResult<isize> {
    sys_linkat(AT_FDCWD, old_path, AT_FDCWD, new_path, 0)
}
```

**建议:** 用一个声明宏生成：
```rust
delegate_legacy_syscall!(link, linkat, old_path, new_path);
delegate_legacy_syscall!(unlink, unlinkat, path);
```

### 2.6 MEDIUM: RISC-V 用户回溯 — 仅 riscv64 实现

**位置:** `os/StarryOS/kernel/src/task/signal.rs:32-93`

RISC-V 实现了完整的 `dump_user_backtrace()`，其他 arch 只有空桩。应将 `dump_user_backtrace` 移到 axcpu 中，每个 arch 可选择实现（空桩或完整回溯）。

### 2.7 LOW: 零星 cfg 门控列表

| 位置 | 类型 | 描述 |
|------|------|------|
| `task/seccomp.rs:75-89` | `AUDIT_ARCH` 常量 | 4 个 arch 各有不同值，可移入 axcpu |
| `axcpu/src/uspace_common.rs:24` | `ExceptionKind::Debug` | x86_64-only 变体，可合并到通用枚举 |
| `kmod/mod.rs:43-188` | LoongArch DMW 分配器 | 已用 `KmodMemBackend` enum 做了合理抽象，低 ROI |
| `perf/hw.rs:1-1196` | ARM PMUv3 | 整个文件 aarch64-only，不存在重复——等有其他 arch 加入 PMU 时再考虑抽象 |
| `mm/loader.rs:26-290+` | RISC-V ELF relocation | 仅 riscv64 有 PIE 重定位支持，无重复 |

---

## 3. 大文件与大型结构体拆分建议

### 3.1 最需拆分的文件（Top 5）

#### #1 `syscall/task/ptrace.rs` **(2,423 行)** — 极度职责过载

单文件混合了 8 种职责：协议常量（73 个）、4 个 arch 的寄存器类型 + 转换逻辑、get/set 寄存器 helpers、信号停等逻辑、seccomp trap 处理、fork/clone/exec/exit 事件上报、syscall 追踪、SEIZE/INTERRUPT 支持。

**拆分方案：**
```
syscall/task/ptrace/
├── mod.rs              ← 主入口 + ioctl 分发 (~500行)
├── constants.rs         ← 所有 PTRACE_* / PTRACE_EVENT_* / PTRACE_O_* 常量
├── regs_riscv64.rs      ← RiscvUserRegs + get/set_regs impl
├── regs_aarch64.rs      ← Aarch64UserRegs + get/set_regs impl
├── regs_loongarch64.rs  ← LoongarchUserRegs + get/set_regs impl
├── regs_x86_64.rs       ← X8664UserRegs + get/set_regs impl
├── stop.rs              ← 信号停等 + syscall 追踪逻辑
├── events.rs            ← fork/clone/exec/exit 事件上报
└── singlestep.rs        ← 单步设置/恢复（下沉到 axcpu 后本文件为空/极薄）
```

#### #2 `task/mod.rs` **(1,942 行)** — God Object 问题

`ProcessData` 结构体（161 行, 30+ 字段）和 `Thread` 结构体（105 行, 30+ 字段）混合了 mm、exec、cred、signal、fd、cgroup、namespace、timer、ptrace、seccomp、futex、perf、job-control 等所有子系统状态。

**拆分方案 — 将 ProcessData 分解为子结构体：**
```rust
pub struct ProcessData {
    // 各子系统抽为独立 Name
    pub mm: MmState,           // address_space, brk, stack, ...
    pub exec: ExecState,        // exec_domain, interp, ...
    pub cred: CredState,        // uid, gid, euid, egid, ...
    pub sig: SignalState,       // sigactions, pending signals, ...
    pub io: IoState,            // fd_table, fs_context(root/cwd), umask
    pub timing: ProcTiming,     // start_time, utime, stime, ...
    pub ns: NsState,            // namespace references
    pub resource: ResourceState, // rlimits, ...
    // 保持顶层简单字段
    pub pid: Pid,
    pub parent_pid: Option<Pid>,
    pub exit_state: ExitState,
    pub job_control: JobControl,
    pub children: Vec<Arc<Thread>>,
}
```

`Thread` 同理，按 concern 拆分为 `ThreadBase` / `ThreadSignal` / `ThreadPtrace` / `ThreadJob`。

#### #3 `pseudofs/proc.rs` **(1,810 行)** — 组合爆炸

~25 个独立的 `/proc` 文件渲染器，每个 30-80 行。拆分极其自然（每个渲染器独立）：

```
pseudofs/proc/
├── mod.rs              ← procfs 树构建器 (~250行)
├── task.rs             ← 进程级文件: status, stat, cmdline, environ, fd/, ns/ (~800行)
├── system.rs           ← 系统级文件: meminfo, cpuinfo, version, loadavg, interrupts, stat, kallsyms
├── seq_file.rs         ← SeqWriter helper
└── render.rs           ← 公共渲染辅助函数
```

#### #4 `pseudofs/dev/card0.rs` **(1,779 行)** — 双模式 DRM 驱动

KMS legacy + atomic 两大模式 + dumb buffer + PRIME + framebuffer 全部挤在一个文件。

```
pseudofs/dev/card0/
├── mod.rs              ← Card0 主 struct + ioctl 分发 + 设备注册 (~400行)
├── dumb_buf.rs         ← CREATE_DUMB, MAP_DUMB, DESTROY_DUMB
├── legacy.rs           ← SETCRTC, PAGE_FLIP, legacy getters
├── atomic.rs           ← MODE_ATOMIC, property 处理
├── props.rs            ← DRM property 元数据初始化
└── fb.rs               ← framebuffer create/destroy/display
```

#### #5 `syscall/fs/ctl.rs` **(829 行)** — 大杂烩

25 个不同类型的 syscall 挤在单文件中（ioctl, fcntl, statfs, chmod, chown, access, mkdir, rmdir, link, unlink, rename, readlink...）。

```
syscall/fs/
├── ctl.rs              ← ioctl, fcntl 核心 (~300行)
├── ctl_stat.rs         ← statfs, fstatfs, stat/fstat/lstat/newfstatat
├── ctl_metadata.rs     ← chmod, chown, access, faccessat, umask
└── ctl_dir.rs          ← mkdir, rmdir, link, unlink, symlink, rename, getdents, readlink
```

### 3.2 其他需要拆分的大文件 (>500 行)

| 优先级 | 文件 | 行数 | 建议拆分为 |
|--------|------|------|-----------|
| **高** | `pseudofs/usbfs/mod.rs` | 1,732 | `ioctl.rs` + `urb.rs` + `ep.rs` + `dev.rs` |
| **高** | `pseudofs/usbfs/manager.rs` | 1,388 | `discovery.rs` + `descriptor.rs` + `refresh.rs` |
| **高** | `perf/hw.rs` | 1,196 | `counting.rs` + `sampling.rs` |
| **高** | `pseudofs/sysfs.rs` | 1,185 | `class.rs` + `bus.rs` + `devices.rs` + `system.rs` |
| **高** | `perf/task.rs` | 1,079 | `sampling.rs` (per-task 采样环) |
| **高** | `syscall/mm/mmap.rs` | 1,060 | `mprotect.rs` + `mremap.rs` + `madvise.rs` |
| **高** | `syscall/fs/io.rs` | 1,011 | `vectored.rs` + `splice.rs` |
| **高** | `pseudofs/dev/tty/terminal/ldisc.rs` | 1,011 | `input.rs` + `echo.rs` (测试移到单独文件) |
| **高** | `syscall/sys.rs` | 1,009 | `syslog.rs` + `random.rs` + `uname_info.rs` + `prctl.rs` |
| **中** | `syscall/fs/aio.rs` | 1,499 | `ring.rs` + `iocb.rs` + `completion.rs` |
| **中** | `syscall/ipc/msg.rs` | 941 | `queue.rs` + `syscall.rs` |
| **中** | `components/axfs-ng-vfs/src/mount.rs` | 920 | `location.rs` + `propagation.rs` |
| **中** | `syscall/fs/lock.rs` | 914 | `posix.rs` + `flock.rs` |
| **中** | `pseudofs/dev/card1.rs` | 873 | `mem.rs` + `submit.rs` |
| **中** | `pseudofs/overlay.rs` | 851 | `dir.rs` + `file.rs` |
| **低** | `syscall/ipc/shm.rs` | 841 | 可保持现状 |
| **低** | `pseudofs/dev/tty/serial.rs` | 814 | `console.rs` |
| **低** | `file/netlink.rs` | 812 | `uevent.rs` + `rtnetlink.rs` |
| **低** | `mm/loader.rs` | 749 | 可保持现状 |
| **低** | `task/ops.rs` | 730 | `exit.rs` + `fork.rs`（可选） |
| **不拆** | `syscall/mod.rs` | 973 | 这是系统调用分发表，保持为一个文件有利于审计 |
| **不拆** | `components/axerrno/src/lib.rs` | 652 | 错误枚举应保持单文件 |
| **不拆** | `pseudofs/dev/drm.rs` | 647 | 纯数据定义，保持为一个 C-header 等价物 |

### 3.3 大型结构体 (body >100 行)

| Struct | 文件 | 行 | 建议 |
|--------|------|-----|------|
| **ProcessData** | `task/mod.rs:611-771` | **161** | 按子系统拆分为 7-8 个子结构体 |
| **AxErrorKind** | `axerrno/src/lib.rs:22-133` | **112** | 保持现状（穷举错误枚举需单文件） |
| **Thread** | `task/mod.rs:97-201` | **105** | 与 ProcessData 类似，按 concern 拆分 |

---

## 4. axbuild 重复逻辑清理与复用

axbuild 源文件约 **220 个, ~22,300 行**。发现了 **11 组**显著的代码重复模式。

### 4.1 优先级 HIGH 的重复

#### #1 三个 OS 的 `board.rs` — 结构完全一致

**文件:** `arceos/board.rs` (208行) / `axvisor/board.rs` (171行) / `starry/board.rs` (226行)

函数签名与实现几乎完全相同：`board_dir()`, `board_default_list()`, `find_board()`, `board_names()`, `load_board_file()`。唯一差异是 board 目录路径和 Board 结构体字段。

**建议:**
```rust
trait BoardDiscovery {
    type Board;
    const BOARD_DIR: &str;
    fn parse_board_file(path: &Path) -> Result<Self::Board>;
}

fn board_default_list<B: BoardDiscovery>(root: &Path) -> Result<Vec<B::Board>> { /* 一次性实现 */ }
```
三个 OS 只需实现 `BoardDiscovery` trait（~20行/OS），消除 ~400 行重复。

#### #2 三个 OS 的 `config.rs` — 结构一致

**文件:** `arceos/config.rs` (193行) / `axvisor/config.rs` (180行) / `starry/config.rs` (344行)

`available_board_names()`, `resolve_board()`, `write_defconfig()` 完全相同的逻辑。

**建议:** 提取 `DefconfigWriter` trait，参数化为 Board 类型和 Snapshot 类型。

#### #3 `load_uboot_config` / `load_board_config` — 3 份副本

**文件:** `arceos/mod.rs:353-386` / `axvisor/mod.rs:354-387` / `starry/mod.rs:440-473`

每个 OS 的 `load_uboot_config` 和 `load_board_config` 方法完全一致（仅使用 `&self.app`）。

**建议:** 将这两个方法移到 `AppContext` 的共享实现中，泛型化为接受 `Option<&Path>` 参数。

#### #4 Qemu/Uboot Snapshot — 6 个形状相同的 struct

**位置:** `context/types.rs:51-152`

```rust
pub struct ArceosQemuSnapshot { pub qemu_config: Option<PathBuf> }  // 重复
pub struct ArceosUbootSnapshot { pub uboot_config: Option<PathBuf> } // 重复
pub struct AxvisorQemuSnapshot { pub qemu_config: Option<PathBuf> }  // 重复
// ... 共 6 个
```

**建议:** 用一个 generic struct `OsSnapshot<T> { config: Option<PathBuf>, _marker: PhantomData<T> }` 替换。或扩展已有的 `impl_snapshot_file!` 宏来覆盖。

### 4.2 优先级 MEDIUM 的重复

#### #5 `prepare_xxx_request` — 三个 OS 遵循相同 8 步流程

**位置:** `context/resolve.rs:35-385`

**建议:** 提取 `resolve_arch_target_from_cli_and_snapshot()` 和 `resolve_runtime_config_paths()` 两个公共 helper。每个 OS 仅需提供特定的 CLI args/snapshot/config 加载函数。

#### #6 Target/Arch 字符串匹配链 — 散落在 7+ 处

**文件:** `build/platform.rs:563-575`, `build/std_build.rs:66-152`, `starry/build.rs:584-613`, `starry/rootfs.rs:280-291` 等

每处都是 `if target.starts_with("x86_64-") else if target.starts_with("aarch64-")...`。

**建议:** `context/arch.rs` 中已有 `ARCH_SPECS` 表。扩展此表以包含 std_target_triple、std_linker_machine、std_c_flags 等字段，然后用一次表查找替换所有 if/else 链。同时让 `starry/rootfs.rs` 的 `rootfs_patch_mode()` 调用 `starry/build.rs` 中的 `uses_dynamic_platform()` 而不是重复 feature 匹配逻辑。

#### #7 `StageLog` 结构重复定义

**位置:** `context/mod.rs:451-473` 和 `starry/build.rs:675-697`

两处完全相同的 `StageLog` struct。

**建议:** 删除 `starry/build.rs` 中的定义，import `crate::context::StageLog`。

#### #8 `command_available` / `command_path` 近似重复

**位置:** `starry/build.rs:539-557` 和 `build/std_build.rs:204-216`

两个函数都遍历 PATH 检查文件是否存在。`command_available` 返回 `bool`，`command_path` 返回 `Option<PathBuf>`。

**建议:** 统一为单个 `command_path()` 函数，`command_available` = `command_path(name).is_some()`。

### 4.3 优先级 LOW 的重复

#### #9 硬编码的 OS 目录路径（95 处 `"os/arceos"` / `"os/StarryOS"` / `"os/axvisor"`）

**建议:** 在 `context/types.rs` 中定义 `ARCEOS_ROOT` / `STARRY_ROOT` / `AXVISOR_ROOT` 常量，通过 path helper 函数派生所有子路径。

#### #10 `impl_snapshot_file!` 宏不完整

**位置:** `context/types.rs:220-240`

该宏已为 3 个 `CommandSnapshot` 生成 `load()` / `store()`，但 6 个 Qemu/Uboot Snapshot 的 `is_empty()` 是手写的。

**建议:** 扩展宏以覆盖 `is_empty()` 生成。

#### #11 测试辅助代码重复

axbuild 测试文件约占 40%（~9,000 行）。`arceos/test/`, `axvisor/test/`, `starry/test/` 中的 `write_workspace` / `write_board` / `write_build_config` helpers 存在大量重复。

---

## 5. 错误处理与 Panic 路径审计

### 5.1 高风险 panics

| 位置 | 行号 | panic 类型 | 触发条件 |
|------|------|-----------|---------|
| `kprobe.rs` | 62,106,113,121,129,137,146,154-155,161,170,174,182-183,187,218,248 | `.pop().unwrap()` / `.try_lock().unwrap()` | kretprobe 栈下溢、锁重入 — **panic 在内核探针路径** |
| `mm/loader.rs` | 334-335,368-370,399,441-443,467,560,590-591 | `.try_into().unwrap()` | 加载畸形 ELF 文件时 panic |
| `kmod/mod.rs` | 205,210 | `.expect("out of memory")` | 内存耗尽时 panic（应对用户态 OOM killer） |
| `syscall/ipc/shm.rs` | 169 | `MappingFlags::from_name("USER").unwrap()` | 字符串查找失败 panic |
| `entry.rs` | 24,30,39,42,50,89,110,114 | `.expect()` in init path | 可接受的初始化 panic |

### 5.2 中风险 panics

| 位置 | 行号 | 问题 |
|------|------|------|
| `pseudofs/dir.rs` | 181 | `self.this.upgrade().unwrap()` — 如果父节点 Arc 被 drop |
| `pseudofs/file.rs` | 61 | `it.unwrap().into()` — `SimpleFileOperation::Read` 结果无错误路径 |
| `uprobe/mod.rs` | 37 | `kprobe::register_uprobe(...).unwrap()` |
| `tracepoint/mod.rs` | 301 | `.get_mut(subsystem_name).unwrap()` — insert 后未保证 key 存在 |
| `cgroup/mod.rs` | 114,146 | `.expect("parent was checked above")` — 假设逻辑不变 |

### 5.3 建议

1. **kprobe/kretprobe 路径:** 将所有 `.unwrap()` 替换为 `Result` 返回。探针路径 panic 是内核致命错误。
2. **ELF 加载器:** `try_into().unwrap()` 应换成 `AxResult` + 明确的 `AxError::BadAddress` / `AxError::NotFound` 返回。
3. **OOM 路径:** `kmod::vmalloc` 应返回 `AxResult` 而非 panic。
4. **`Arc::upgrade().unwrap()` 模式:** 使用 `Arc::upgrade().ok_or(AxError::NotFound)?` 替代。
5. **整体建议:** 建立 lint 规则 — 在 kernel/src 中不允许非测试代码中直接使用 `unwrap()` / `expect()`（init 路径除外）。

---

## 6. 模块边界与可见性

### 6.1 全局重导出问题

| 位置 | 问题 |
|------|------|
| `syscall/mod.rs:20-23` | `pub use self::{fs::*, io_mpx::*, ipc::*, mm::*, net::*, ns::*, resources::*, signal::*, sync::*, sys::*, task::*, time::*}` — 全局通配符重导出，无法审计暴露面 |
| `starry-signal/src/lib.rs:11-17` | `pub use action::*`, `pub use pending::*`, `pub use types::*` — 与 syscall 类似 |

**建议:** `starry-signal` 改为显式命名导出。`syscall/mod.rs` 的 glob re-export 可以通过拆分 syscall 模块后每个子系统只 re-export syscall 函数来解决。

### 6.2 白璧微瑕

- `starry-vm/src/lib.rs` 和 `starry-process/src/lib.rs` 的 re-export 模式是 clean 的 — 精确导出具体类型。
- kernel `lib.rs` 中大部分模块是 `mod` (private)，对外仅暴露了 `dyn_debug`, `entry`, `kprobe` 三个 `pub mod`，边界控制较好。

---

## 7. 依赖分析

### 7.1 Kernel Cargo.toml — 可精简的依赖 (~8 个)

| 依赖 | 问题 | 建议 |
|------|------|------|
| `chrono = "0.4"` | 全量日期时间库，内核级用太重 | 使用 core 类型 + 简单时间计算替代 |
| `rand = "0.10"` | RNG crate，仅用于 `getrandom()` syscall | 用硬件 RNG 指令 + 最小包装替代 |
| `ouroboros = "0.18"` | 自引用结构 proc-macro 库，依赖重 | 评估是否真的需要，或用 unsafe + 手动生命周期替代 |
| `flatten_objects = "0.2.4"` | 用途不明确 | 审计是否可以移除 |
| `inherit-methods-macro = "0.1.0"` | 极小、边缘 crate | 检查是否可以用标准模式替代 |
| `bitmaps = "3.2"` | 独立的 bitset 库 | 检查是否与 `bitflags` 功能冗余 |
| `slab = "0.4.9"` | 与 hashbrown 分配器可能冗余 | 评估是否可以统一 |
| `tock-registers = "0.9"` | 版本 pin 0.9 与 workspace 0.10 冲突 | 升级 sg200x-bsp 以兼容 0.10 或统一版本 |

### 7.2 Feature flag 传递链

`sg2002-wifi` feature 启用 `sg2002`，间接拉入 `ax-dma`, `axklib`, `sg2002-tpu`, `tock-registers`。这个传递链建议明确文档化，避免意外依赖膨胀。

### 7.3 组件间耦合

```
starry-kernel
  ├── starry-process  ← 干净的依赖
  ├── starry-signal   ← 依赖 starry-vm（通过 VmPtr/VmMutPtr）
  └── starry-vm       ← 干净的
```

`starry-signal → starry-vm` 的耦合点（信号处理需要访问用户态内存）是合理的，但如果将来 split 更多，考虑提取一个 `UserMemoryAccess` trait 到 `starry-vm` 的公共接口中。

---

## 8. TODO/FIXME/HACK 积压

### 8.1 总计: 54 处, 分布在 25 个文件中

### 8.2 按优先级分类

**FIXME (缺陷级) — 7 处：**

| 文件 | 行号 | 内容 |
|------|------|------|
| `syscall/resources.rs` | 36 | `FIXME: AnyBitPattern` — 可能的内存安全 UB |
| `syscall/time.rs` | 112 | `FIXME: AnyBitPattern` |
| `syscall/task/ctl.rs` | 70 | `FIXME: AnyBitPattern` |
| `syscall/task/schedule.rs` | 49 | `FIXME: AnyBitPattern` |
| `syscall/fs/stat.rs` | 212 | `FIXME: Zeroable` |
| `syscall/fs/ctl.rs` | 273 | `FIXME: safety` |
| `mm/aspace/backend/shared.rs` | 83 | `FIXME: This implementation does not allow map or unmap partial ranges.` |

这些 `AnyBitPattern` / `Zeroable` FIXME 表明存在 unsafe 传参依赖 `bytemuck` 的 trait bound，但未正式实现，存在 UB 风险。

**TODO (功能缺口) — 主要积压：**

- **`syscall/net/opt.rs`** 有 19 个未实现的 socket option（TCP_NODELAY, SO_RCVTIMEO, SO_SNDTIMEO 等）
- **`pseudofs/file.rs:340`** — `TODO: create a linux like seq file` (procfs seq 文件基础设施)
- **`syscall/mm/mmap.rs:575`** — `TODO: implement PROT_GROWSUP & PROT_GROWSDOWN`
- **`syscall/ipc/shm.rs:586`** — `TODO: solve shmflg: SHM_RND and SHM_REMAP`
- **`entry.rs:103`** — `TODO: wait for all processes to finish` (shutdown 路径)

---

## 9. 死代码与未使用导入

### 9.1 Kernel src 中

- `#[allow(dead_code)]` — **8 处**：`file/wext.rs:256`, `file/netlink.rs:263,424`, `file/mod.rs:162`, `kmod/kprint.rs:21`, `pseudofs/dev/card0.rs:1775`, `syscall/ipc/shm.rs:390`
- `#[allow(unused)]` — **3 处**：`pseudofs/proc.rs:1604`, `perf/mod.rs:515,521`

### 9.2 组件中

- `someboot/src/` — 约 28 处 `#[allow(dead_code)]`，主要集中在 loongarch64 arch 代码（context.rs, pte.rs, register）— 表明 LoongArch 移植不完整
- `rsext4/src/crc32c/` — 5 处 ARM64 特定的 CRC32 实现未使用

**建议:** 在 CI 中加入 `cargo clippy -- -W dead-code -W unused-imports`（全局警告）来阻止新的死代码积累。逐步清理现有的 `allow` 注释。

---

## 10. 测试覆盖率缺口

### 10.1 覆盖率概览

| 指标 | 数值 |
|------|------|
| 总源文件 | **189** |
| 含 `#[cfg(test)]` 的文件 | **11** (5.8%) |
| 专用测试文件 | **1** (`axtest_kernel.rs`) |

### 10.2 关键未测试模块

| 模块 | 行数 | 风险 |
|------|------|------|
| `syscall/task/ptrace.rs` | 2,423 | **极高** — 最复杂的子系统，零测试 |
| `task/mod.rs` | 1,942 | **高** — 核心进程/线程管理，仅 2 个测试 |
| `pseudofs/usbfs/mod.rs` | 1,732 | **高** — USB 设备交互，零测试 |
| `pseudofs/dev/card0.rs` | 1,779 | **中** — DRM 模拟设备，零测试 |
| `syscall/fs/aio.rs` | 1,499 | **中** — AIO 子系统，零测试 |
| `syscall/mod.rs` | 973 | **中** — 系统调用分发表，零测试 |
| `syscall/fs/lock.rs` | 914 | **中** — 文件锁含死锁检测，零测试 |
| `mm/loader.rs` | 749 | **极高** — ELF 加载器，安全关键，零测试 |
| `syscall/ipc/shm.rs` | 841 | **中** — SysV 共享内存，零测试 |

### 10.3 建议

1. **最高优先级:** `mm/loader.rs` — 加载畸形 ELF 不应 panic。添加 fuzz-style 测试（随机字节 → 应返回错误，不 panic）。
2. **高优先级:** `syscall/task/ptrace.rs` — 至少加入每个 PTRACE 命令的 smoke 测试。
3. **中优先级:** `syscall/fs/lock.rs` — 文件锁死锁检测逻辑需要确定性测试。
4. **流程改进:** 对于所有 new feature PR，要求在 test-suit 中添加对应的系统级集成测试。

---

## 11. 大 Match 语句与字符串化模式

### 11.1 系统调用分发 (~880 行 match)

**位置:** `syscall/mod.rs:82-966`

**现状:** 每个 arm 是 `Sysno::xxx => sys_xxx(uctx.arg0() as _, uctx.arg1() as _, ...)`。这是机械性重复但也是标准的 dispatch 模式。

**建议:** 不需要拆分文件，但可以考虑用 proc-macro 自动生成 dispatch table（如 `#[syscall_handler]` attribute 标记函数 + `generate_dispatch_table!()` 展开为 match）。

### 11.2 字符串化权限 (50+ 处内联八进制)

15+ 个文件中反复出现：
```rust
NodePermission::from_bits_truncate(0o755)
NodePermission::from_bits_truncate(0o644)
NodePermission::from_bits_truncate(0o444)
// ...
```

**建议:** 在一次性的权限模块中定义常量：
```rust
pub mod perm {
    pub const RUSR_WUSR_XUSR_RGRP_ROTH: NodePermission = NodePermission::from_bits_truncate(0o744);
    // 或更好的使用位运算组合
}
```

### 11.3 原始类型强制转换 (100+ 处裸 `as`)

- `fd as usize` (文件描述符转换, 无边界检查)
- `arg0() as _, arg1() as _` (syscall 参数转换)
- `tv_sec as u64, tv_nsec as u32` (时间结构体转换, 7 种不同的 timeval-like struct 各自转换)

**建议:** 为关键转换点（fd, timeval, syscall args）引入 checked conversion helpers。syscall arg 转换统一用公共 helper 函数，避免在每个 syscall 函数中手工 `as` 转换。

### 11.4 `InterfaceKind` 属性分发

**位置:** `file/netlink.rs:563-573`

```rust
ty: match info.kind { InterfaceKind::Loopback => ARPHRD_LOOPBACK, InterfaceKind::Ethernet => ARPHRD_ETHER }
```

**建议:** 在 `InterfaceKind` 上实现 `fn arphrd(&self) -> u16`，替换散落的 match。

---

## 12. 行动建议优先级矩阵

### P0 — 应立即处理（安全与正确性）

| # | 行动 | 影响范围 | 风险降低 |
|---|------|---------|---------|
| 1 | kprobe/kretprobe 路径 panic → Result | `kprobe.rs` | 消除内核探针路径 panic |
| 2 | ELF 加载器 panic → 错误返回 | `mm/loader.rs` | 防止恶意 ELF 导致内核崩溃 |
| 3 | 所有 `AnyBitPattern` / `Zeroable` / `safety` FIXME 审计 | 6 个文件 | 消除潜在 UB |
| 4 | `mm/aspace/backend/shared.rs` 部分 map/unmap 限制修复 | `shared.rs` | 修复已知功能缺陷 |

### P1 — 高优先级重构（代码可维护性）

| # | 行动 | 预计节省 | 难度 |
|---|------|---------|------|
| 5 | FP save/restore for ptrace 下沉到 axcpu | ~200 行重复消除 | 中 |
| 6 | `dump_user_crash_context` 下沉到 axcpu | ~70 行重复消除 | 低 |
| 7 | ptrace singlestep setup 下沉到 axcpu | ~200 行重复消除 | 中 |
| 8 | `ptrace.rs` 拆分 (2,423行 → 6-8 文件) | 可维护性飞跃 | 中 |
| 9 | `task/mod.rs` 拆分 + ProcessData/Thread 分解 | God Object 消除 | 高 |
| 10 | axbuild 三个 `board.rs` 统一 | ~400 行重复消除 | 中 |
| 11 | axbuild 三个 `config.rs` 统一 | ~100 行重复消除 | 中 |
| 12 | axbuild `load_uboot_config`/`load_board_config` 统一 | ~80 行重复消除 | 低 |

### P2 — 中优先级改进

| # | 行动 | 预计节省 |
|---|------|---------|
| 13 | `proc.rs` 拆分渲染器分类 | 可维护性 |
| 14 | `card0.rs` 拆分 legacy/atomic/dumb_buf | 可维护性 |
| 15 | `syscall/fs/ctl.rs` 拆分为 stat/metadata/dir | 可维护性 |
| 16 | `syscall/sys.rs` 拆分 syslog/random/prctl | 可维护性 |
| 17 | axbuild 6 个 snapshot struct → 1 个 generic | ~60 行 |
| 18 | axbuild `prepare_xxx_request` 统一 8 步流程 | ~150 行 |
| 19 | axbuild `ARCH_SPECS` 表扩展, 消除 target 字符串匹配链 | ~40 行 |
| 20 | `starry/rootfs.rs` `rootfs_patch_mode` 复用 `uses_dynamic_platform` | ~10 行 |
| 21 | `StageLog` 重复定义消除 | ~25 行 |
| 22 | `command_available` / `command_path` 统一 | ~15 行 |

### P3 — 低优先级 / 渐进式

| # | 行动 |
|---|------|
| 23 | 内联八进制权限替换为命名常量 (~50 处) |
| 24 | `InterfaceKind` 方法化（消除散落的 match） |
| 25 | 系统调用参数转换 helper 统一 |
| 26 | axbuild OS 路径硬编码 → 常量 + helper |
| 27 | 清理 `#[allow(dead_code)]` / `#[allow(unused)]` |
| 28 | 通配符 `pub use ::*` → 显式导出 |
| 29 | 内核依赖精简（chrono, rand, ouroboros, flatten_objects 等） |
| 30 | `tock-registers` 版本冲突解决 |

### P4 — 基础设施/流程

| # | 行动 |
|---|------|
| 31 | CI 中启用 `dead_code` / `unused_imports` lint 全局警告 |
| 32 | PR 要求：大文件 (>500行) 不接受 new PR，必须先拆分 |
| 33 | PR 要求：non-init-path `unwrap()` / `expect()` 必须有文档或替换 |
| 34 | 为 `mm/loader.rs`, `syscall/task/ptrace.rs` 添加确定性 fuzz/单元测试 |
| 35 | 在 test-suit 中为每个关键子系统添加 smoke 级集成测试 |

---

## 附录 A: 完整大文件清单 (>500 行)

| 文件 | 行数 | 优先级 |
|------|------|--------|
| `syscall/task/ptrace.rs` | 2,423 | P1 |
| `task/mod.rs` | 1,942 | P1 |
| `pseudofs/proc.rs` | 1,810 | P2 |
| `pseudofs/dev/card0.rs` | 1,779 | P2 |
| `pseudofs/usbfs/mod.rs` | 1,732 | P2 |
| `syscall/fs/aio.rs` | 1,499 | P2 |
| `pseudofs/usbfs/manager.rs` | 1,388 | P2 |
| `perf/hw.rs` | 1,196 | P2 |
| `pseudofs/sysfs.rs` | 1,185 | P2 |
| `perf/task.rs` | 1,079 | P2 |
| `syscall/mm/mmap.rs` | 1,060 | P2 |
| `syscall/fs/io.rs` | 1,011 | P2 |
| `pseudofs/dev/tty/terminal/ldisc.rs` | 1,011 | P2 |
| `syscall/sys.rs` | 1,009 | P2 |
| `syscall/mod.rs` | 973 | **不拆** |
| `syscall/ipc/msg.rs` | 941 | P3 |
| `components/axfs-ng-vfs/src/mount.rs` | 920 | P3 |
| `syscall/fs/lock.rs` | 914 | P3 |
| `pseudofs/dev/card1.rs` | 873 | P3 |
| `pseudofs/overlay.rs` | 851 | P3 |
| `syscall/ipc/shm.rs` | 841 | P3 |
| `syscall/fs/ctl.rs` | 829 | P2 |
| `pseudofs/dev/tty/serial.rs` | 814 | P3 |
| `file/netlink.rs` | 812 | P3 |
| `mm/loader.rs` | 749 | P3 |
| `task/ops.rs` | 730 | P3 |
| `mm/aspace/mod.rs` | 717 | P3 |
| `file/epoll.rs` | 705 | P3 |
| `task/signal.rs` | 697 | P3 |
| `syscall/fs/fd_ops.rs` | 681 | P3 |
| `task/futex.rs` | 655 | P3 |
| `pseudofs/dev/event.rs` | 654 | P3 |
| `components/axerrno/src/lib.rs` | 652 | **不拆** |
| `pseudofs/dev/drm.rs` | 647 | **不拆** |
| `pseudofs/tmp.rs` | 640 | P3 |
| `mm/access.rs` | 637 | P3 |
| `components/axcpu/src/aarch64/pmu.rs` | 621 | **不拆** |
| `components/axcpu/src/loongarch64/unaligned.rs` | 612 | P3 |
| `perf/sampling.rs` | 611 | P3 |
| `mm/aspace/backend/cow.rs` | 605 | P3 |
| `pseudofs/dev/mod.rs` | 602 | **不拆** |
| `perf/mod.rs` | 570 | **不拆** |
| `syscall/task/ctl.rs` | 561 | P3 |
| `pseudofs/dev/kpu.rs` | 559 | P3 |
| `pseudofs/dev/tpu/device.rs` | 541 | P3 |
| `kprobe.rs` | 533 | P3 |
| `pseudofs/dev/tty/usb_serial/mod.rs` | 527 | P3 |
| `syscall/task/clone.rs` | 522 | P3 |
| `file/memfd.rs` | 520 | P3 |
| `mm/aspace/backend/file.rs` | 519 | P3 |
| `syscall/net/opt.rs` | 514 | P3 |
| `syscall/task/wait.rs` | 509 | P3 |

---

## 附录 B: axbuild 文件大小排名 (Top 15)

| 文件 | 行数 |
|------|------|
| `spin_lint.rs` | 1,090 |
| `build/platform.rs` | 784 |
| `build/std_build.rs` | 737 |
| `arceos/mod.rs` | 737 |
| `starry/kmod.rs` | 741 |
| `starry/mod.rs` | 672 |
| `starry/build.rs` | 700 |
| `sync_lint/parser.rs` | 701 |
| `context/mod.rs` | 646 |
| `starry/quick_start.rs` | 623 |
| `image/storage.rs` | 583 |
| `axloader/mod.rs` | 572 |
| `rootfs/qemu.rs` | 556 |
| `axvisor/rootfs.rs` | 520 |
| `context/resolve.rs` | 461 |

---

*报告生成完成。各模块的具体拆分和重构顺序建议与相关 maintainer 协商后确定。*
