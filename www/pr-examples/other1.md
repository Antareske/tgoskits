# fix(starry): 修复 futex 与 robust-list 语义并补充回归测例

## 概述

本 PR 修复了 StarryOS futex / robust-list 相关的多个 Linux 兼容性问题，并新增一组专门的用户态 syscall 回归测例。

本 PR 覆盖：

- `futex(2)` wait / wake 的超时与取消清理语义。
- `FUTEX_WAIT_BITSET` / `FUTEX_WAKE_BITSET` 参数校验。
- `FUTEX_REQUEUE` 后 waiter 生命周期安全。
- robust-list owner death 的 Linux ABI 行为。
- 新增 grouped syscall 测例二进制：`/usr/bin/test-futex-robust-list`。

## 测例设计

新增测例位置：

`test-suit/starryos/normal/qemu-smp1/syscall/test-futex-robust-list/c/`

测例使用 C 语言直接调用 raw `syscall()`，不依赖 glibc robust mutex 封装，因此验证的是 StarryOS syscall ABI 本身。

### 复用 upstream 的内容

> 参考的 upstream 测例来自：
> https://github.com/rcore-os/linux-compatible-testsuit
> `rcore-os/linux-compatible-testsuit/tests/test_futex.c`

upstream 主要覆盖：

- 通过 `MAP_SHARED | MAP_ANONYMOUS` 与 `fork()` 验证跨进程 `FUTEX_WAIT` / `FUTEX_WAKE`。
- `FUTEX_WAIT` 相对超时返回 `ETIMEDOUT`。
- futex word 当前值不匹配时返回 `EAGAIN` / `EWOULDBLOCK`。
- 没有 waiter 时 `FUTEX_WAKE` 返回 `0`。
- 多 waiter 场景下 `FUTEX_WAKE(n)` 的唤醒数量。
- `FUTEX_WAIT_PRIVATE` / `FUTEX_WAKE_PRIVATE` 基础行为。
- 非法用户指针。
- 超时时长 sanity check。

本 PR 复用了这些测试思路，并将其融合进本地 StarryOS grouped syscall 测例框架。

### 相对 upstream 的改进

本地测例做了以下增强：

- forked wake-count 测例使用有界 futex wait，避免调度竞态导致子进程永久睡眠。
- 最终清理阶段先修改 futex word，再 `FUTEX_WAKE`，晚到的子进程会通过 `EAGAIN` 返回，而不是无界阻塞。
- 共享计数使用原子访问，避免普通共享内存读写带来的可见性问题。
- 通过 `waitpid()` 检查子进程退出状态，子进程异常会被报告为测例失败。
- robust-list ABI 测例使用静态 robust head，避免主线程退出时扫描已失效的栈地址。

### 本地额外覆盖

本 PR 的本地测例还覆盖了 upstream 未覆盖的内容：

| 类别 | 覆盖内容 |
| --- | --- |
| `FUTEX_WAIT_BITSET` / `FUTEX_WAKE_BITSET` | mask 不相交时不唤醒；<br>mask 相交时唤醒；<br>绝对时间在过去时返回 `ETIMEDOUT`；<br>`val3 == 0` 返回 `EINVAL`。 |
| `set_robust_list` / `get_robust_list` ABI | valid head + correct size；<br>`get_robust_list(0, ...)`；<br>invalid size；<br>invalid output pointer；<br>nonexistent tid。 |
| robust-list owner death | owner 线程注册 robust list；<br>waiter 等待 owner TID 对应的 futex word；<br>owner 线程退出；<br>waiter 被唤醒；<br>futex word 出现 `FUTEX_OWNER_DIED`；<br>futex word 的 owner TID bits 被清零。 |
| `FUTEX_REQUEUE` | `FUTEX_REQUEUE` waiter identity collision 回归测试。 |

该测例已加入 `qemu-smp1/syscall/qemu-*.toml`，覆盖 x86_64、riscv64、aarch64、loongarch64 的 grouped syscall 配置。

## 测例发现的问题

### 1. `FUTEX_WAIT` 超时后可能留下 stale waiter

根因：

旧的 wait queue 只保存 `(Waker, bitset)`。当 `future::timeout()` 先完成时，被 drop 的 wait future 没有办法从队列中删除属于自己的 waiter entry。

影响：

后续 `FUTEX_WAKE` 可能唤醒并计数一个已经没有任务等待的 stale entry。这样 stale waiter 会消耗 wake 名额，真实 waiter 可能错过唤醒。

### 2. `FUTEX_REQUEUE` 后 waiter ID 可能在目标队列中碰撞

根因：

旧设计如果使用队列局部 ID 来清理 waiter，那么 `FUTEX_REQUEUE` 把 waiter 从源队列移动到目标队列后，该 ID 不再保证在目标队列内唯一。目标队列中的另一个 waiter 超时或被取消时，可能按相同 ID 删除掉无关的 requeued waiter。

影响：

`FUTEX_REQUEUE` / `FUTEX_CMP_REQUEUE` 的典型用户包括 pthread condition variable。waiter 被误删后可能错过后续 wake，造成阻塞甚至死锁。

### 3. `WAIT_BITSET` / `WAKE_BITSET` 未拒绝 `val3 == 0`

根因：

Linux 要求 bitset futex 操作的 bitset 非零。StarryOS 之前没有对 `value3 == 0` 做校验。

影响：

非法 bitset 操作可能被当成无效 mask 的 wait/wake 处理，而不是按 Linux ABI 返回 `EINVAL`。

### 4. robust-list owner death 使用了内核内部状态，而不是更新用户态 futex word

根因：

旧实现通过 futex entry 内部的 `owner_dead` 标记记录 owner death，并让后续 `FUTEX_WAIT` 返回 `EOWNERDEAD`。但 Linux robust-list 语义要求内核直接更新用户态 futex word：设置 `FUTEX_OWNER_DIED`，清除 TID bits，然后唤醒一个 waiter。

影响：

用户态 robust mutex 实现通常检查 futex word 本身。旧实现不会让用户态看到 `FUTEX_OWNER_DIED`，与 Linux ABI 不兼容。

## 修复方案

### 使用 per-waiter state 作为 waiter 身份

`WaitQueue` 现在保存结构化 waiter：

- `Waker`
- bitset
- `Arc<WaiterState>`

wait future 自己也持有同一个 `Arc<WaiterState>`。因此在 timeout / interruption / drop 时，它可以：

- 标记自身为 cancelled；
- 使用 `Arc::ptr_eq` 只删除自己的队列 entry。

这是最小修复，因为它没有引入全局 waiter ID，也没有改变 futex table 的组织方式。waiter 身份跟随 waiter 对象本身移动，所以经过 `FUTEX_REQUEUE` 后仍然有效。

### wake 路径清理 cancelled waiter

`wake()` 现在会：

- 跳过并清理 cancelled waiter；
- 在实际 wake 前把 waiter state 标记为 woken；
- 只删除实际被唤醒的 waiter。

`is_empty()` 也会顺便清理 cancelled waiter。

### bitset 参数校验

`sys_futex` 现在对 `FUTEX_WAIT_BITSET` 和 `FUTEX_WAKE_BITSET` 检查 `value3 == 0`，并返回 `EINVAL`。

### robust-list owner death 对齐 Linux ABI

`handle_futex_death()` 现在会：

- 读取用户态 futex word；
- 检查 TID bits 是否匹配正在退出的 owner；
- 写回 `(old_value & !FUTEX_TID_MASK) | FUTEX_OWNER_DIED`；
- 唤醒 futex queue 上的一个 waiter。

旧的 futex entry 内部 `owner_dead` 标记和 `FUTEX_WAIT` 返回 `EOWNERDEAD` 路径被移除。这样修复范围集中在 Linux ABI 要求的用户态 futex word 状态上。

## 为什么这是最小修复

- 没有引入新的 futex 全局 ID 分配器。
- 没有改变 futex key / futex table 的整体结构。
- 没有扩展到 PI futex、`WAKE_OP` 等未覆盖功能。
- robust-list 只修正 owner death 的 ABI 可见状态，不额外改变 robust mutex 的用户态协议。
- 测例只新增一个 focused syscall 子测例，并接入现有 grouped syscall 测试框架。

## 已完成验证

已在 Docker 容器内执行：

| 验证项 | 结果 |
| --- | --- |
| `cargo fmt` | 通过 |
| `starry-kernel` clippy | 全部配置通过 |

已在最终版本上完成 x86_64、riscv64、aarch64、loongarch64 的 focused syscall grouped case 验证：

| 架构 | 命令 | 结果 |
| --- | --- | --- |
| x86_64 | `cargo xtask starry test qemu --arch x86_64 -g normal -c syscall 2>&1 \| tee target\futex-robust-list-qemu-x86_64.log` | 通过 |
| riscv64 | `cargo xtask starry test qemu --arch riscv64 -g normal -c syscall 2>&1 \| tee target\futex-robust-list-qemu-riscv64.log` | 通过 |
| aarch64 | `cargo xtask starry test qemu --arch aarch64 -g normal -c syscall 2>&1 \| tee target\futex-robust-list-qemu-aarch64.log` | 通过 |
| loongarch64 | `cargo xtask starry test qemu --arch loongarch64 -g normal -c syscall 2>&1 \| tee target\futex-robust-list-qemu-loongarch64.log` | 通过 |

日志显示：

| 日志项 | 结果 |
| --- | --- |
| `/usr/bin/test-futex-robust-list` | `DONE: 64 pass, 0 fail` |
| grouped syscall case | `STARRY_GROUPED_TESTS_PASSED` |
| summary | `failed (0)` |

最后的 bounded-wait 测例 polish 已执行静态检查：

```powershell
git diff --check
```

结果通过。
