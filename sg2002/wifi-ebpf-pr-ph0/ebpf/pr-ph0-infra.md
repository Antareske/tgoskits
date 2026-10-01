# PR：eBPF 探针基础设施补强

标题（英文，供提交使用）：

```
feat(starry-kernel,ax-cpu): keep a faulty eBPF probe from taking the kernel down
```

---

## 背景

StarryOS 的 eBPF 子系统目前有三处会让"挂一个探针"变成能带走内核的操作：

- `kbpf-basic` 提供的 `bpf_probe_read`（helper 4）是普通 memcpy 且恒返回成功。地址未映射时直接陷入页错误；地址可访问但偏移已过期时静默读到垃圾。探针读的是随版本漂移的内核结构，两种失败都不罕见。
- `kbpf-basic` 的 preprocessor 会遍历 `bpf(2)` 交给内核的指令流，并且在读宽加载之后的槽位之前不检查该槽是否存在。指令流直接来自用户态，因此一个畸形程序就是任何进程都能索取的内核 panic。
- 挂点注册失败（入口指令无法重定位）时直接 panic。

探针是排障手段，这三类失败都不该由内核承担。前两类一旦发生还很难取证：内核卡死之后日志不再输出。

## 方案

### 1. 故障安全的内核地址拷贝

`ax-cpu` 新增 `kernel-access` 特性与 `copy_from_kernel_nofault(dst, src, len)`：四个架构各有一段汇编，把逐字节的 load 与 store 都登记进 nofault 异常表，地址不可访问时控制流跳到恢复标签，函数返回尚未拷贝的字节数。该表在异常进入操作系统处理之前被查询，因此在关中断上下文里也能拿到失败结果而不触发页错误路径。x86_64 上非规范地址触发 `#GP` 而不是 `#PF`，通用保护处理程序因此也在 panic 之前查询同一张表。

### 2. 探针读改走故障安全路径

覆盖 helper 表项 4 与 113（后者是 aya 读取内核内存时发出的 id），两者由同一实现提供。读失败返回 `-EFAULT` 并把目标清零，成功返回 0，与 Linux 约定一致；长度超过 4096 直接返回 `-EINVAL`——这里没有 verifier 帮忙限制长度，而 helper 可能落在关中断上下文里。

### 3. 指令流在进入 preprocessor 之前先做结构校验

`load_prog` 在预处理之前按 preprocessor 自己的走法过一遍指令流：宽加载之后的槽位必须存在，并且只有当源字段是要被重定位的 map（`BPF_PSEUDO_MAP_FD` / `BPF_PSEUDO_MAP_VALUE`）时，该槽才被当作这条宽加载的一部分，否则它就是下一条指令。畸形流因此在加载期以 `EINVAL` 被拒，而不是在 preprocessor 里 panic。判定用的是加载期，探针路径上不增加成本。

### 4. 挂点注册失败报错而不是 panic

`register_kprobe` / `register_kretprobe` 返回 `StarryError::InvalidInput`（`EINVAL`）并记录被拒绝的挂点，`perf_event_open` 路径把它传给用户态。选择 `EINVAL` 而不是 `ENOSYS`：拒绝的原因是调用者挑的地址放不了断点，不是内核缺少 kprobe 能力；按后者处理会让调用方把这个能力整会话关掉。

## 改动范围

| 区域 | 内容 |
| --- | --- |
| `components/axcpu` | `kernel-access` 特性、`kernel_access.rs`、四个架构的 `kernel_copy.S`、x86_64 `#GP` 恢复分支 |
| `os/StarryOS/kernel` | helper 4/113 的实现替换、`ebpf/verify.rs` 的结构校验与 `load_prog` 的调用、kprobe 注册的错误返回 |
| `os/StarryOS/lkm` | kprobe 测试模块按拒绝结果分支 |
| `test-suit/arceos/cpu/user-entry` | 无故障内核拷贝用例；既有缺页恢复用例改用同一页表守卫 |
| 文档 | `test-suit/arceos/cpu/README.md` 的用例契约、`docs/design/ax-cpu-validation.md` 的阶段记录 |

## 验证

### 静态检查与单元测试

- `cargo xtask clippy --package starry-kernel --package ax-cpu`：通过。
- `cargo xtask test --since dev`：17 个包全部通过。其中 starry-kernel 的单测覆盖：结构校验对缺少 operand 槽的宽加载、含非重定位宽加载对的流、与不足一条指令的尾部的拒绝，以及对良构流的放行；helper 在长度上限处的整段拷贝与超限拒绝。
- `cargo fmt --check`：通过。

### QEMU

- `cargo xtask arceos test qemu --arch aarch64 --test-group cpu`：9/9 用例通过。新增的无故障内核拷贝用例在两个已映射区间之间整段拷贝成功；装入空用户页表后，以不可访问的源与不可访问的目的地各调用一次，两次都返回 `KernelAccessError::Fault`，且源不可访问时目的地保持原内容。用例输出 `CPU_KERNEL_ACCESS_OK`，位于既有用户 PMU 用例之前；组内既有用例保持通过。
- 既有 eBPF 应用在 riscv64 上的加载与挂载冒烟：`ebpf/mytrace`（tracepoint 路径；其程序含子程序调用与循环，见"未覆盖"一项）与 `ebpf/syscall_count`（kprobe 路径）。两者都正常加载、挂载并运行，没有出现 `bpf prog rejected`。

**未覆盖**

- 解释器的执行时间没有上限，本改动不处理这一点。"让程序本身有界"曾计划做成加载期判定，但真实 aya 程序既有子程序调用（`mytrace` 的程序里就有），也有循环（它的循环来自 aya 的字符串读取 helper，逐字节读到 NUL），任何拒绝这些构造的判据都会把既有应用全部拒掉；可行方向是运行期步数预算或真正的循环分析，其中前者需要给 `rbpf` 的 `execute_program` 加预算参数（该接口目前没有），二者都超出本次范围。
- 系统调用级回归：`perf_event_open(PERF_TYPE_KPROBE)` 对无法承载断点的地址改返回 `EINVAL`、`BPF_PROG_LOAD` 对畸形指令流新增拒绝，这两条用户可见行为没有对应的 StarryOS 系统用例。
- 非 AArch64 目标的恢复分支：四架构汇编均参与构建与静态检查，但 x86_64 的 `#GP` 恢复分支与 LoongArch、RISC-V 的恢复分支本轮没有真实执行。
- 实体目标：本轮未在实体板卡上执行。

## 兼容性与迁移

- `register_kprobe` / `register_kretprobe` 的返回类型由 `Arc<..>` 变为 `StarryResult<Arc<..>>`，树内调用方（`perf_event_open` 路径与 lkm 测试模块）已一并更新。
- 加载期新增的拒绝只针对畸形指令流：合法但包含循环或子程序调用的程序不受影响。
- 性能：探针路径上只多一次长度比较与一次读取实现替换；未采样的帧不增加任何时钟读取。
- 可观测性：被拒的程序与被拒的挂点都写入内核日志并带上拒绝原因。
- 回滚：三个提交各自独立；单独回滚第二个提交会把 helper 表退回故障不安全的实现，单独回滚第三个提交会退回挂点注册 panic。
