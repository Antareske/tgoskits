# sigsetsize Linux 兼容性全面分析

## 实测环境

- 内核：x86_64 Linux（本机）
- glibc `sizeof(sigset_t)` = **128**
- `_NSIG` = 65，`kernel_sigset_size` = `_NSIG/8` = **8**

---

## 一、Linux 内核实测结果（原始数据）

### 规律一：所有 syscall，non-NULL mask，sigsz 必须精确等于 8

```
signalfd4 / epoll_pwait / epoll_pwait2 / ppoll / rt_sigprocmask
  sigsz=0   → EINVAL  ✗
  sigsz=4   → EINVAL  ✗
  sigsz=7   → EINVAL  ✗
  sigsz=8   → OK      ✓
  sigsz=9   → EINVAL  ✗
  sigsz=16  → EINVAL  ✗
  sigsz=128 → EINVAL  ✗
```

**结论：Linux 对 sigsetsize 的校验是精确等于 `_NSIG/8 = 8`，不多不少。**

### 规律二：NULL mask 的 syscall，sigsz 完全不校验

```
epoll_pwait(NULL mask) / epoll_pwait2(NULL mask) / ppoll(NULL mask)
  sigsz=0   → OK  ✓
  sigsz=8   → OK  ✓
  sigsz=16  → OK  ✓
  sigsz=128 → OK  ✓
```

`signalfd4` 不存在"NULL mask"路径（mask 是必填的指针，传 NULL 会 EFAULT）。
`rt_sigprocmask` 的 sigsetsize 即使 set/oset 都 NULL 也必须是 8。

---

## 二、man 手册关键条款

### epoll_wait(2) — C library/kernel differences

> The raw `epoll_pwait()` and `epoll_pwait2()` system calls have a sixth
> argument, `size_t sigsetsize`, which specifies the size in bytes of the
> sigmask argument. **The glibc `epoll_pwait()` wrapper function specifies
> this argument as a fixed value (equal to `sizeof(sigset_t)`).**

手册此处的 `sizeof(sigset_t)` 指的是 **glibc wrapper 内部传给内核的值**，
即 `sizeof(kernel_sigset_t) = 8`，而非用户空间头文件里的 `sizeof(sigset_t) = 128`。
这是手册的措辞歧义，不是内核接受 128。

### poll(2) — C library/kernel differences (ppoll)

> The raw `ppoll()` system call has a fifth argument, `size_t sigsetsize`,
> which specifies the size in bytes of the sigmask argument. **The glibc
> `ppoll()` wrapper function specifies this argument as a fixed value
> (equal to `sizeof(kernel_sigset_t)`).**

此处手册直接写了 `sizeof(kernel_sigset_t)`，无歧义。

### signalfd(2) — C library/kernel differences

> The underlying Linux system call requires an additional argument,
> `size_t sizemask`, which specifies the size of the mask argument.
> **The glibc `signalfd()` wrapper function does not include this argument,
> since it provides the required value for the underlying system call.**

glibc 自动填入 8，用户无法干预。

---

## 三、对 StarryOS 改动的逐项分析

### 3.1 `check_sigset_size` 修改（signal.rs）

**改动前（PR #250 引入）：**
```rust
if size != 0 && size < size_of::<SignalSet>() {
    return Err(AxError::InvalidInput);
}
// 接受：0, 8, 9, 16, 128, ...
```

**改动后（当前 dev）：**
```rust
if size != size_of::<SignalSet>() {
    return Err(AxError::InvalidInput);
}
// 接受：仅 8
```

**与 Linux 对齐？**

| sigsz | Linux | 改动前 StarryOS | 改动后 StarryOS |
|------:|:-----:|:---------------:|:---------------:|
| 0     | EINVAL | OK（错误）      | EINVAL ✓        |
| 4     | EINVAL | EINVAL ✓        | EINVAL ✓        |
| 7     | EINVAL | EINVAL ✓        | EINVAL ✓        |
| 8     | OK     | OK ✓            | OK ✓            |
| 9     | EINVAL | OK（错误）      | EINVAL ✓        |
| 16    | EINVAL | OK（错误）      | EINVAL ✓        |
| 128   | EINVAL | OK（错误）      | EINVAL ✓        |

**直接结论：改动后与 Linux 完全对齐。改动是正确的。**

### 3.2 `sys_ppoll` NULL mask 路径修改（poll.rs）

**改动前：**
```rust
check_sigset_size(sigsetsize)?;  // 无论 mask 是否 NULL 都校验
```

**改动后：**
```rust
if !sigmask.is_null() {
    check_sigset_size(sigsetsize)?;
}
```

**与 Linux 对齐？**

| mask | sigsz | Linux | 改动前 StarryOS | 改动后 StarryOS |
|:----:|------:|:-----:|:---------------:|:---------------:|
| NULL | 0     | OK    | EINVAL（错误）  | OK ✓            |
| NULL | 16    | OK    | OK（碰巧对）    | OK ✓            |
| NULL | 128   | OK    | OK（碰巧对）    | OK ✓            |
| non-NULL | 8  | OK  | OK ✓            | OK ✓            |
| non-NULL | 16 | EINVAL | OK（错误）  | EINVAL ✓        |

**直接结论：改动后与 Linux 完全对齐。**

---

## 四、三个失败测例的合理性分析

### 4.1 `test-epoll-pwait2`：2 处 FAIL

**失败位置：**
```c
// main.c:112
CHECK_RET(raw_epoll_pwait2(epfd, out, 4, &ts, &mask, sizeof(sigset_t)), 0,
          "sigmask + sizeof(sigset_t) accepted");

// main.c:263
long r_masked = raw_epoll_pwait2(epfd, out, 1, &ts, &block_usr1, sizeof(sigset_t));
CHECK(r_masked == 0, "masked SIGUSR1 does not interrupt epoll_pwait2 timeout");
```

两处都传 `sizeof(sigset_t) = 128`，期望返回 0（成功）。

**Linux 实测：sigsetsize=128, non-NULL mask → EINVAL**

**直接结论：测例预期与 Linux 不对齐。测例错误，需修正为 `sigsetsize=8`。**

### 4.2 `test-epoll-pwait-sigsetsize`：1 处 FAIL

**失败位置：**
```c
// main.c:66
CHECK_RET(raw_epoll_pwait(epfd, out, 1, &ts_poll, &mask, 16), 0,
          "sigmask + size=16 (musl) accepted");
```

传 `sigsetsize=16`，期望被接受（PR #250 专门为 musl 兼容性新增的测试）。

**Linux 实测：sigsetsize=16, non-NULL mask → EINVAL**

**直接结论：测例预期与 Linux 不对齐。测例本身就是错误地对齐了 PR #250 的错误假设，需删除或修正该子测试。**

### 4.3 `test-signalfd4`：1 处 FAIL

**失败位置：**
```c
// main.c:135
int sfd0 = (int)syscall(SYS_signalfd4, -1, &mask, (size_t)0, 0);
CHECK(sfd0 >= 0, "sigsetsize=0 succeeds");
```

传 `sigsetsize=0`，期望创建成功。

**Linux 实测：signalfd4, sigsz=0 → EINVAL**

**直接结论：测例预期与 Linux 不对齐。`sigsetsize=0` 在 Linux 上是无效值，测例需修正。**

---

## 五、需要修正的测例汇总

| 测例 | 失败行 | 当前错误预期 | 正确的 Linux 行为 | 修正方向 |
|:-----|-------:|:------------|:-----------------|:---------|
| `test-epoll-pwait2` | 112 | `sigsetsize=128` 返回 0 | EINVAL | 改用 `sigsetsize=8` |
| `test-epoll-pwait2` | 263 | `sigsetsize=128` 返回 0 | EINVAL | 改用 `sigsetsize=8` |
| `test-epoll-pwait-sigsetsize` | 66 | `sigsetsize=16` 被接受 | EINVAL | 删除或改为期望 EINVAL |
| `test-signalfd4` | 135 | `sigsetsize=0` 被接受 | EINVAL | 改为期望 EINVAL |

---

## 六、总结

PR #250 的出发点是对的（musl 兼容性），但对 Linux 内核行为的判断有误：
musl 的 libc wrapper 自己会把 `sigsetsize` 截成 8 再传内核，内核从不收到 16。
内核对 `sigsetsize` 的要求是**全局一致的精确等于 8**，适用于所有相关 syscall，
任何偏离（0, 9, 16, 128）都返回 EINVAL。

当前 dev 分支对 `check_sigset_size` 和 `sys_ppoll` 的修改，
是将 StarryOS 行为对齐 Linux man 手册及内核实测的**正确修复**。
唯一的后续工作是修正上述 4 处测例，使测例本身也对齐 Linux 语义。
