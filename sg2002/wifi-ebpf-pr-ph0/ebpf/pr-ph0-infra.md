# PR：eBPF 探针基础设施补强

标题（英文，供提交使用）：

```
feat(starry-kernel,ax-cpu): keep a faulty eBPF probe from crashing the kernel
```

---

## 背景

StarryOS 的 eBPF 子系统目前有三处会使加载探针的操作可能导致内核崩溃：

- `kbpf-basic` 提供的 `bpf_probe_read`（helper 4）是普通 memcpy 且恒返回成功。地址未映射时直接陷入页错误；地址可访问但偏移已失效时静默读取到错误内容。探针读取的是随版本变化的内核结构，两类失败都可能发生。
- `kbpf-basic` 的 preprocessor 会遍历 `bpf(2)` 交给内核的指令流，并且在读取宽加载之后的槽位之前不检查该槽是否存在。指令流直接来自用户态，因此畸形的指令流可以由任意进程构造，并触发内核 panic。
- 挂点注册失败（入口指令无法重定位）时直接 panic。

探针用于诊断，这三类失败都不应由内核承担。前两类一旦发生，现场也难以取证：内核停止运行之后日志不再输出。

## 方案

### 1. 故障安全的内核地址拷贝

`ax-cpu` 新增 `kernel-access` 特性与 `copy_from_kernel_nofault(dst, src, len)`：四个架构各有一段汇编，将逐字节的 load 与 store 登记进 nofault 异常表，地址不可访问时控制流跳转到恢复标签，函数返回尚未拷贝的字节数。该表在异常进入操作系统处理之前被查询，因此在关中断上下文中也能得到失败结果，而不进入页错误处理路径。

不可翻译的地址并不都产生页错误：x86_64 上是 `#GP`，AArch64 上是地址长度故障，LoongArch 上是访存地址错误。这三类都被各自的分派送往致命路径，因此都在调用致命路径之前查询同一张表，四个架构的缺页路径同样如此。

这些恢复分支由 `exception-table` 特性控制，而不是 `kernel-access`——存在异常表即在陷阱处理之前查表是组件级约定，既有的缺页分支已经遵循这一约定。因此打开该特性的同类构建都会进入该分支，其中既有的用户地址拷贝在遇到非规范地址时也从 panic 变为返回 `EFAULT`，同属这一约定的延伸。

### 2. 探针读取改为故障安全路径

覆盖 helper 表项 4 与 113（后者是 aya 读取内核内存时发出的 id），两者由同一实现提供。读失败返回 `-EFAULT` 并将目标缓冲区清零，成功返回 0，与 Linux 约定一致；长度超过 4096 返回 `-EINVAL`——本实现没有 verifier 参与限制长度，而 helper 可能在关中断上下文中执行。

### 3. 指令流在进入 preprocessor 之前先做结构校验

`load_prog` 在预处理之前按 preprocessor 自身的遍历方式走一遍指令流：宽加载之后的槽位必须存在，并且只有当源字段是要被重定位的 map（`BPF_PSEUDO_MAP_FD` / `BPF_PSEUDO_MAP_VALUE`）时，该槽才被当作这条宽加载的一部分，否则它就是下一条指令。畸形流因此在加载期以 `EINVAL` 被拒，而不是在 preprocessor 内 panic。该判定在加载期完成，探针执行路径上没有额外开销。

panic 的根因在依赖内部：`kbpf-basic` 的 `EbpfPreProcessor::preprocess` 直接索引宽加载之后的那条指令，`rbpf` 的 `to_insn_vec` 在长度不是指令宽度整数倍时 panic。该检查在调用方边界处拦截，两个依赖本身未作改动；因此校验逻辑需要与 preprocessor 的语义保持一致，依赖改变判据时此处需要同步更新。

### 4. 挂点注册失败返回错误而不是 panic

`register_kprobe` / `register_kretprobe` 返回 `StarryError::InvalidInput`（`EINVAL`）并记录被拒绝的挂点，`perf_event_open` 路径将其传给用户态。选择 `EINVAL` 而不是 `ENOSYS`：拒绝的原因是调用者选择的地址无法放置断点，不是内核缺少 kprobe 能力；按后者处理会使调用方在本会话中关闭该能力。

## 改动范围

| 区域 | 内容 |
| --- | --- |
| `components/axcpu` | `kernel-access` 特性、`kernel_access.rs`、四个架构的 `kernel_copy.S`、x86_64 `#GP` 与 AArch64/LoongArch 非页错误故障的恢复分支 |
| `os/StarryOS/kernel` | helper 4/113 的实现替换、`ebpf/verify.rs` 的结构校验与 `load_prog` 的调用、kprobe 注册的错误返回 |
| `os/StarryOS/kernel/build.rs` | 宿主测试构建时为镜像链接脚本提供的符号补定义 |
| `os/StarryOS/lkm` | kprobe 测试模块按注册结果分支 |
| `test-suit/arceos/cpu/user-entry` | 无故障内核拷贝用例，AArch64 / RISC-V / LoongArch 三个配置；既有缺页恢复用例改用同一页表守卫 |
| 文档 | `test-suit/arceos/cpu/README.md` 的用例契约、`docs/design/ax-cpu-validation.md` 的阶段记录 |

## 验证

### 静态检查与单元测试

- `cargo xtask clippy --since dev`：选中 6 个包、185 项检查，全部通过；`ax-cpu` 的 `kernel-access` 组合在四个架构上各检查一遍，用例包含 AArch64、RISC-V 与 LoongArch 三个裸机目标。
- `cargo xtask test --since dev`：18 个包全部通过。其中 starry-kernel 的单测覆盖：结构校验对缺少 operand 槽的宽加载、含非重定位宽加载对的流、与不足一条指令的尾部的拒绝，以及对良构流的放行；helper 在长度上限处的整段拷贝与超限拒绝。
- 同一入口在 `CARGO_INCREMENTAL=0` 下再跑一次同样全部通过。宿主测试二进制的链接在非增量构建下与增量构建不同：非增量把多个模块的汇编并入一个代码段，宿主链接因此要求镜像链接脚本定义的符号，构建脚本为宿主目标补齐了它们。
- `cargo fmt --check`：通过。

### QEMU

- `cargo xtask arceos test qemu --arch aarch64 --test-group cpu`：9/9 用例通过。新增的无故障内核拷贝用例在两个已映射区间之间整段拷贝成功；装入空用户页表后，以不可访问的源与不可访问的目的地各调用一次，两次都返回 `KernelAccessError::Fault`，且源不可访问时目的地保持原内容。用例输出 `CPU_KERNEL_ACCESS_OK`，位于既有用户 PMU 用例之前；组内既有用例保持通过。
- `cargo xtask arceos test qemu --arch riscv64 --test-group cpu`：7/7 用例通过。RISC-V 没有可清空的用户地址段寄存器，用例改用对 Sv39、Sv48、Sv57 均非规范的地址取得同一个结果；恢复分支因此在 RISC-V 上也得到运行证据。该用例进入 CPU 组的 RISC-V 批次后由持续集成持续执行。
- `cargo xtask arceos test qemu --arch loongarch64 --test-group cpu`：新增配置后 1/1 通过。该地址在 LoongArch 上产生访存地址错误，补上恢复之前用例确定性失败（`Unhandled trap Exception(MemoryAccessAddressError) … BADV=0xfe00000000000000`），补上之后通过。AArch64 的 QEMU 模型对同一地址给出的是 level-0 翻译故障，所以该架构在修复前后都通过。
- 在 riscv64 上运行既有 eBPF 应用：`ebpf/syscall_count`（kprobe 路径）加载、挂载并实际执行，输出 `SYSCALL_COUNT_PASS: 736 records across 6 syscall ids`；其程序通过 `TracePointContext::read_at` 读取内核内存，正是本改动替换的 helper，因此这次执行同时说明替换后的读取路径在 riscv64 上按预期工作。`ebpf/mytrace`（tracepoint 路径；其程序含子程序调用与循环，见"未覆盖"一项）正常加载、挂载，没有出现 `bpf prog rejected`。

**未覆盖**

- 解释器的执行时间没有上限，本改动不处理这一点。"使程序本身有界"曾计划做成加载期判定，但真实 aya 程序既有子程序调用（`mytrace` 的程序中即存在），也有循环（其循环来自 aya 的字符串读取 helper，逐字节读取到 NUL），任何拒绝这些构造的判据都会使既有应用无法加载；可行方向是运行期步数预算或循环分析，其中前者需要给 `rbpf` 的 `execute_program` 增加预算参数（该接口目前没有），二者都超出本次范围。
- 系统调用级回归：`perf_event_open(PERF_TYPE_KPROBE)` 对无法承载断点的地址改返回 `EINVAL`、`BPF_PROG_LOAD` 对畸形指令流新增拒绝，这两条用户可见行为没有对应的 StarryOS 系统用例。
- x86_64 的 `#GP` 恢复分支：四架构汇编均参与构建与静态检查，AArch64、RISC-V 与 LoongArch 的恢复分支已实际执行，x86_64 因非规范地址产生的 `#GP` 恢复分支本轮没有运行，该用例目前也没有 x86_64 配置。
- 实体目标：本轮未在实体板卡上执行。

## 兼容性与迁移

- `register_kprobe` / `register_kretprobe` 的返回类型由 `Arc<..>` 变为 `StarryResult<Arc<..>>`，树内调用方（`perf_event_open` 路径与 lkm 测试模块）已一并更新。
- 加载期新增的拒绝只针对畸形指令流：合法但包含循环或子程序调用的程序不受影响。
- `ax-cpu` 一侧为纯新增：新特性默认关闭，未打开它的构建没有行为变化。打开 `exception-table` 的 x86_64 构建会有上述 `#GP` 分支的变化。
- 构建：构建脚本只在宿主目标下为那 5 个符号补定义；内核镜像仍由链接脚本提供，镜像构建与链接方式不变。
- 性能：探针路径上只多一次长度比较与一次读取实现替换；未采样的帧不增加任何时钟读取。
- 可观测性：被拒的程序与被拒的挂点都写入内核日志，并带有拒绝原因。
- 回滚：三部分改动可以分别回退。单独回滚拷贝原语会使 helper 表退回不支持故障恢复的实现，单独回滚挂点注册会恢复为 panic；用例与文档的提交可以单独回退，不影响被测代码。
