# net_stats eBPF 真实字节计数实现方案

## 背景与目标

net_stats eBPF 程序在所有架构上字节计数器恒为 0，包计数器正常。当前 HEAD 采用 `packets × 64` 估算值作为规避方案，并在 README/TODO 中记为“已知限制”。

本方案的目标：以真实字节数替代估算值，让 TCP/UDP 的 send/recv 字节计数反映实际传输量。

## 误诊纠正

个人文档 `www/net-stats-fix-summary.md` 给出的结论有两处错误，均已通过反汇编
`target/x86_64-unknown-linux-musl/release/starryos` 证伪。

### 错误一：sret 指针的读取寄存器

`AxError` 定义为 `#[repr(transparent)] struct AxError(i32)`，故
`AxResult<usize>` = `Result<usize, i32>`，16 字节。

反汇编 `<TcpSocket as SocketOps>::send`（`0xffffffff800e9430`）确认走 sret 约定：

```
; 入口：隐藏 sret 指针（rdi）存入 rbp
ffffffff800e9447: mov %rdi,%rbp
; 尾声：sret 指针经 rbp 写回 rax 后返回
ffffffff800e966f: mov %rbp,%rax
...
ffffffff800e9683: ret
```

即**函数返回瞬间，sret 指针在 RAX**。

被回退的实现（提交 `acfc4515d`）却用 `ctx.arg::<u64>(0)` 读该指针——`arg(0)` 对应
RDI，仅在函数入口有效。返回时 RDI 已被多次覆盖（`mov %rdx,%rdi`、
`lea 0x28(%r14),%rdi` 等）。结果是解引用了陈旧指针，其判别式字非 0，
`read_ok_bytes_from_sret` 每次 `return None`，字节计数恒为 0。

**正确做法：kretprobe 中从 `ctx.ret()`（RAX）读取 sret 指针。**

### 错误二：bpf_probe_read 的失败模式

本 VM 中 `bpf_probe_read` / `bpf_probe_read_kernel`（kbpf-basic 0.6.0 的
`raw_bpf_probe_read`）实现为一句 `dst.copy_from_slice(src)`，无地址校验、无缺页处理，
不可能“返回 None”，只会成功或触发缺页。summary 中“尝试 1 返回 None”的观察不成立。

### 附带问题：探测符号过宽

loader 的片段 `["6ax_net3tcp","9TcpSocket","9SocketOps4send"]` 在 kallsyms 中匹配
19 个局部符号（kallsyms 保留局部 `t` 符号），包含：

- `Future::poll` 包装（返回布局不同的 `Poll<AxResult<usize>>`）
- `block_on` / `send_impl` / `enqueue_many` 等

对这些非 `Result<usize, AxError>` 返回类型套用 sret 布局读取字节会得到错误值，
且一次逻辑 send 命中多层符号，污染包计数。需过滤到规范单态化 trait 方法。

## ABI 与运行时机制（已核实）

- **返回寄存器映射**（aya `ret()` = `rc_reg()`）：x86_64=rax，aarch64=x0，riscv64=a0。
- **sret 布局**：`[+0] u64 判别式（0=Ok，非零=Err）`，`[+8] u64 字节数（Ok 时有效）`。
- **kretprobe 保存时机**：`kprobe-0.6.0` 的 x86 trampoline 在 RESTORE 之前
  `pushq %rax`，`trapframe_to_ptregs`（`os/StarryOS/kernel/src/kprobe.rs`）保存 rax/rdx。
  eBPF context 为 `kprobe::PtRegs` 原样传入（`perf/bpf.rs` 的 `execute_with_ptregs`），
  其字段顺序镜像 Linux `pt_regs`，故 aya 的偏移正确。
- **跨架构一致性**：aarch64 canonical send 入口 `mov x20, x8`（x8 为 AAPCS64 的 sret
  寄存器），尾声 `mov x0, x20; ret`，与 x86_64 同形。

## 实现步骤

### 1. eBPF 侧：`apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs`

- 恢复真实读取函数，从返回寄存器取 sret 指针：

```rust
const MAX_IO_BYTES: u64 = 1 << 30;

#[inline(always)]
fn read_ok_bytes_from_ret(ctx: &RetProbeContext) -> Option<u64> {
    // sret 指针位于返回寄存器（x86_64: RAX / aarch64: X0 / riscv64: A0）
    let ptr = ctx.ret::<u64>() as *const u64;
    if ptr.is_null() {
        return None;
    }
    // 判别式：0 = Ok，非零 = Err
    let disc = unsafe { bpf_probe_read_kernel(ptr).ok()? };
    if disc != 0 {
        return None;
    }
    // 字节数位于 +8
    let bytes = unsafe { bpf_probe_read_kernel(ptr.add(1)).ok()? };
    (bytes <= MAX_IO_BYTES).then_some(bytes)
}
```

- 删除 `ESTIMATED_AVG_PACKET_SIZE` 常量、各架构的估算分支，以及“KNOWN LIMITATION”长注释块。
- 四个 `*_ret` kretprobe 调用点保持不变（已在调用 `read_ok_bytes_from_ret`）。

### 2. loader 侧：`apps/starry/ebpf/net_stats/net_stats/src/main.rs`

- 在 `resolve_symbols` 之后加入规范方法过滤，仅保留 canonical trait 方法，
  排除包装/内联闭包符号：

```rust
fn is_canonical_socketop(sym: &str) -> bool {
    const WRAPPERS: &[&str] = &[
        "poll_fn", "block_on", "send_impl", "recv_impl",
        "Future", "6future", "enqueue", "dequeue", "ring_buffer", ".llvm.",
    ];
    !WRAPPERS.iter().any(|w| sym.contains(w))
}
```

  对四组 `syms_*` 应用 `.retain(is_canonical_socketop)`（或在 `resolve_symbols`
  内过滤）。已验证过滤后每组恰好留下规范单态化方法。

### 3. 测试

- `--test` 的非零校验此时才具备实义（真实读取失败会真的为 0 并使测试失败）。
- QEMU `success_regex` 维持 `TEST PASSED`，四个架构 TOML 不变。

### 4. 文档回改（正式文档，需对齐）

- `apps/starry/ebpf/net_stats/README.md`：
  - 功能列表恢复为真实 byte counters（去掉 “estimated” 限定）。
  - Known Limitations 中的“字节为估算值”一节改写为：字节数经 sret 指针（返回寄存器）
    真实读取；如需保留说明，改为描述 ABI 依赖而非“无法读取”。
- `apps/starry/net-bench/docs/TODO.md`：
  - “已完成”节的字节计数条目改为描述真实读取实现。
  - 中优先级里“实现真实字节统计”一条标记为完成或移除。

### 5. 验证

- 重建 x86_64 与 aarch64。
- `cargo xtask starry app qemu --test-case ebpf/net_stats --arch x86_64`
  （及 aarch64），确认四个 byte counter 非零，且量级与自测流量相符
  （不再是 pkts × 64 的固定整数倍）。

## 风险

- 依赖 kretprobe trampoline 在返回瞬间保存 RAX。机制已核实成立，但最终以 QEMU
  实跑的非零结果为准。
- loongarch64 存在 QEMU virtio 崩溃（与本改动无关），字节读取正确性以其余三架构为准。

## 关键符号地址（x86_64，供复核）

- canonical `<TcpSocket as SocketOps>::send::<ReadBuf>`：`0xffffffff800e9430`
- canonical `<TcpSocket as SocketOps>::recv::<WriteBuf>`：`0xffffffff800e9320`
