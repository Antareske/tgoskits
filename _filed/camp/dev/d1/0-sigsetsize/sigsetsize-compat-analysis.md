# sigsetsize 兼容性分析：PR #250 前后改动及 Linux 非兼容问题

## 背景

`check_sigset_size` 是 StarryOS 内核中所有接受 `sigsetsize` 参数的 syscall 的统一校验函数，
位于 `os/StarryOS/kernel/src/syscall/signal.rs`。

调用方共 8 处：

| 调用位置 | 文件 |
|---|---|
| `sys_rt_sigprocmask` | `syscall/signal.rs` |
| `sys_rt_sigaction` | `syscall/signal.rs` |
| `sys_rt_sigpending` | `syscall/signal.rs` |
| `sys_rt_sigsuspend` 等其余 signal.rs 调用 | `syscall/signal.rs` |
| `do_epoll_wait`（`!sigmask.is_null()` 时） | `syscall/io_mpx/epoll.rs` |
| `sys_ppoll` | `syscall/io_mpx/poll.rs` |
| `do_select`（sigmask 非 NULL 时） | `syscall/io_mpx/select.rs` |
| `sys_signalfd4` | `syscall/fs/signalfd.rs` |

---

## PR #250 之前的实现

commit `f562db864` 及更早版本中，校验逻辑为**精确等于**：

```rust
// 旧代码
if size != size_of::<SignalSet>() && size != 0 {
    return Err(AxError::InvalidInput);
}
```

`size_of::<SignalSet>()` = 8（`SignalSet` 是 `u64`，即内核的 `kernel_sigset_t`）。

接受的值：`0` 或 `8`，其余全部返回 `EINVAL`。

---

## PR #250 的改动（commit `05d77c96b`）

**标题**：`fix(epoll): fix epoll_pwait sigsetsize incompatibility with musl`

**问题描述**：musl libc 将 `sigset_t` 定义为 128 位（`_NSIG = 128`，`sizeof(sigset_t) = 16`），
因此 musl 程序通过 libc wrapper 调用 `epoll_pwait` 时，wrapper 传入的 `sigsetsize = 16`。
旧代码对 `16` 返回 `EINVAL`，导致 musl 程序无法正常使用 epoll 的信号屏蔽功能。

**修改后的逻辑**：

```rust
// PR #250 引入的代码
// Accept the kernel sigset size and any larger libc/ABI sigset size,
// since the kernel only uses the low `size_of::<SignalSet>()` bytes
// (glibc uses 8, musl uses 16). Keep accepting 0 for callers that use
// it to mean "no mask".
if size != 0 && size < size_of::<SignalSet>() {
    return Err(AxError::InvalidInput);
}
```

接受的值：`0` 或任意 `>= 8` 的值（包括 `16`、`128` 等）。

**PR #250 同时新增了测试** `test-epoll-pwait-sigsetsize`，验证 `size=8`（glibc）和 `size=16`（musl）
均被接受，`size=4` 被拒绝。

---

## 问题：与 Linux 内核实际行为不符

### Linux 内核的真实校验

Linux 内核对所有接受 `sigsetsize` 的 syscall 均执行**精确等于**校验：

```c
/* Linux fs/eventpoll.c, kernel/signal.c 等处的统一模式 */
if (sigsetsize != sizeof(sigset_t))
    return -EINVAL;
```

其中 `sizeof(sigset_t)`（内核侧）= `_NSIG / 8` = `64 / 8` = **8 字节**。

### 实测结果（x86_64 Linux 6.x）

```
epoll_pwait2  sigsetsize=8    => ret=0   errno=0   ✓
epoll_pwait2  sigsetsize=16   => ret=-1  errno=22  ✗ EINVAL
epoll_pwait2  sigsetsize=128  => ret=-1  errno=22  ✗ EINVAL

ppoll         sigsetsize=8    => ret=0   errno=0   ✓
ppoll         sigsetsize=16   => ret=-1  errno=22  ✗ EINVAL

rt_sigprocmask sigsetsize=8   => ret=0   errno=0   ✓
rt_sigprocmask sigsetsize=16  => ret=-1  errno=22  ✗ EINVAL

signalfd4     sigsetsize=8    => ret=fd  errno=0   ✓
signalfd4     sigsetsize=16   => ret=-1  errno=22  ✗ EINVAL
```

**NULL sigmask 时**：Linux 不校验 `sigsetsize`（`ppoll NULL mask size=128` 返回 0）。

### musl 兼容性的真相

PR #250 的前提假设——"musl wrapper 会把 `sigsetsize=16` 传给内核"——是错误的。

musl 的 `epoll_pwait` wrapper 实现（`src/event/epoll_pwait.c`）在调用 `SYS_epoll_pwait`
时传入的是 `_NSIG / 8`（即 8），而不是 `sizeof(sigset_t)`（16）。这是 libc wrapper 的
标准职责：将用户空间的 ABI 类型转换为内核期望的参数。

因此：
- **通过 musl libc wrapper 调用**：传给内核的 `sigsetsize` 始终是 8，不存在兼容性问题。
- **静态链接 musl 并绕过 wrapper 裸调 syscall**：传 16 在 Linux 上也会得到 `EINVAL`，
  StarryOS 接受 16 反而制造了与 Linux 的行为差异。

### 由此引发的具体测试失败

`test-epoll-pwait2` 在 Linux 上失败，在 StarryOS 上通过，原因正是此处：

```c
// test-epoll-pwait2/c/src/main.c 第 111 行
CHECK_RET(raw_epoll_pwait2(epfd, out, 4, &ts, &mask, sizeof(sigset_t)), 0,
          "sigmask + sizeof(sigset_t) accepted");
// sizeof(sigset_t) = 128（glibc），Linux 返回 EINVAL，StarryOS 返回 0
```

```c
// 第 263 行（同样原因）
long r_masked = raw_epoll_pwait2(epfd, out, 1, &ts, &block_usr1, sizeof(sigset_t));
CHECK(r_masked == 0, "masked SIGUSR1 does not interrupt epoll_pwait2 timeout");
```

---

## 额外问题：`sys_ppoll` 的 NULL mask 路径

`sys_ppoll`（`syscall/io_mpx/poll.rs`）在 PR #250 前后均无条件调用 `check_sigset_size`，
即使 `sigmask` 为 NULL。Linux 内核在 `sigmask=NULL` 时不校验 `sigsetsize`。

```rust
// 修复前（有问题）
pub fn sys_ppoll(...) -> AxResult<isize> {
    check_sigset_size(sigsetsize)?;  // NULL mask 时也校验，与 Linux 不符
    ...
}
```

---

## 当前修复（dev 分支）

### 1. `check_sigset_size` 恢复精确等于语义

```rust
// os/StarryOS/kernel/src/syscall/signal.rs
pub(crate) fn check_sigset_size(size: usize) -> AxResult<()> {
    // [原始注释保留]
    // Accept the kernel sigset size and any larger libc/ABI sigset size,
    // since the kernel only uses the low `size_of::<SignalSet>()` bytes
    // (glibc uses 8, musl uses 16). Keep accepting 0 for callers that use
    // it to mean "no mask".
    //
    // NOTE(linux-compat): The reasoning above is correct in theory, but Linux
    // kernel enforces an exact match: sigsetsize must equal sizeof(kernel_sigset_t)
    // = _NSIG/8 = 8. Every syscall that takes a sigsetsize (rt_sigprocmask,
    // rt_sigaction, epoll_pwait, epoll_pwait2, ppoll, signalfd4, …) rejects any
    // value other than 8 with EINVAL, including values larger than 8 such as the
    // musl sizeof(sigset_t) = 16 or the glibc sizeof(sigset_t) = 128.
    // musl's libc wrappers normalise sigsetsize to _NSIG/8 before entering the
    // kernel, so a conforming musl binary never passes 16 to the raw syscall.
    // Accepting sizes > 8 therefore diverges from Linux without providing real
    // musl compatibility, and causes test-epoll-pwait2 to fail on Linux while
    // passing on StarryOS (the two FAIL cases at lines 111 and 263 of that test).
    if size != size_of::<SignalSet>() {
        return Err(AxError::InvalidInput);
    }
    Ok(())
}
```

### 2. `sys_ppoll` 仅在 sigmask 非 NULL 时校验

```rust
// os/StarryOS/kernel/src/syscall/io_mpx/poll.rs
pub fn sys_ppoll(...) -> AxResult<isize> {
    // Linux only validates sigsetsize when sigmask is non-NULL; a NULL sigmask
    // makes ppoll behave like poll and the sigsetsize argument is ignored.
    if !sigmask.is_null() {
        check_sigset_size(sigsetsize)?;
    }
    ...
}
```

---

## 影响评估

| 场景 | 修复前 | 修复后（对齐 Linux） |
|---|---|---|
| glibc 程序通过 wrapper 调用（sigsetsize=8） | ✓ 通过 | ✓ 通过 |
| musl 程序通过 wrapper 调用（wrapper 传 8） | ✓ 通过 | ✓ 通过 |
| 裸 syscall sigsetsize=16 | ✓ 通过（错误） | ✗ EINVAL（对齐 Linux） |
| 裸 syscall sigsetsize=128 | ✓ 通过（错误） | ✗ EINVAL（对齐 Linux） |
| ppoll NULL mask 任意 sigsetsize | 取决于值 | ✓ 不校验（对齐 Linux） |
| test-epoll-pwait2 在 Linux 上 | ✗ 失败 | ✓ 通过 |
| test-epoll-pwait-sigsetsize（StarryOS） | ✓ 通过 | 需重新评估（size=16 用例将失败） |

`test-epoll-pwait-sigsetsize` 中 `size=16` 的用例（Test 4）在修复后会失败，
因为该测试本身的预期与 Linux 内核行为不符，需要同步修正。
