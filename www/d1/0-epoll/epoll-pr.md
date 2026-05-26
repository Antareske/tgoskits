## 背景

本 PR 在 StarryOS syscall 测试套件中补充 epoll 相关 syscall 的 Linux 兼容性测例，覆盖 `epoll_create1`、`epoll_ctl`、`epoll_pwait`、`epoll_pwait2` 四个 syscall。

## 新增测例

### test-epoll

测例位置：`test-suit/starryos/normal/qemu-smp1/syscall/test-epoll/`

直接移植自 [rcore-os/linux-compatible-testsuit](https://github.com/rcore-os/linux-compatible-testsuit) 的 `test_epoll.c`，基于 pipe 验证 epoll 的核心语义，共 12 个 PART：

| PART | 覆盖内容 |
| --- | --- |
| 1–2 | `epoll_create1(0)` 和 `epoll_create1(EPOLL_CLOEXEC)`，验证 `FD_CLOEXEC` 标志 |
| 3 | `epoll_ctl ADD` + `epoll_wait` 基本 `EPOLLIN` 事件检测 |
| 4 | `epoll_wait` 无事件时超时返回 0 |
| 5 | `epoll_ctl MOD`，修改监控事件类型（`EPOLLIN` → `EPOLLOUT`） |
| 6 | `epoll_ctl DEL`，删除后 `epoll_wait` 不再报告该 fd |
| 7 | 管道写端关闭后 `EPOLLHUP` 事件 |
| 8 | 多 fd 同时监控，两个 pipe 同时就绪返回 2 |
| 9 | `EPOLLET` 边沿触发语义 |
| 10 | 负向：无效 fd 返回 `EBADF`/`EINVAL`，重复 ADD 返回 `EEXIST`，MOD/DEL 不存在的 fd 返回 `EBADF`/`ENOENT` |
| 11 | `fork` 后子进程继承 epoll 实例，子进程写入后父进程 `epoll_wait` 正常返回 |
| 12 | `epoll_ctl DEL` 已关闭的 fd，验证内核自动清理行为 |

### test-epoll-pwait2

测例位置：`test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-pwait2/`

通过 `syscall(SYS_epoll_pwait2, ...)` 直接调用内核，验证 `epoll_pwait2` 的 Linux ABI 语义：

| 测试函数 | 覆盖内容 |
| --- | --- |
| `test_invalid_args` | 无效 epfd 返回 `EBADF`；`maxevents == 0` 和 `maxevents < 0` 返回 `EINVAL` |
| `test_null_timeout_and_zero_timeout` | `timeout={0,0}` 立即返回 0；`timeout=NULL` 阻塞直到 fd 就绪 |
| `test_timeout_and_sigsetsize` | `NULL sigmask` 忽略 `sigsetsize`；`sigsetsize` 过小返回 `EINVAL` |
| `test_efault_events_ptr` | 非法 `events` 指针返回 `EFAULT` |
| `test_timeout_monotonic_duration` | 100ms 超时实际等待时长符合预期（≥60ms，≤1000ms） |
| `test_maxevents_boundary` | `maxevents=1` 时只返回一个就绪事件，剩余事件可在后续调用中获取 |
| `test_sigmask_effect_and_eintr` | 临时信号掩码屏蔽 `SIGUSR1` 时不被中断；未屏蔽时被信号中断返回 `EINTR` |

### CI 接入

两个测例均已加入 `qemu-smp1/syscall/qemu-{x86_64,aarch64,riscv64,loongarch64}.toml`，纳入 grouped syscall case，通过 `STARRY_GROUPED_TESTS_PASSED` 判定。

## 已完成验证

已在四个架构上执行 `cargo xtask starry test qemu --target <target> -c syscall`：

| 架构 | 命令 | 结果 |
| --- | --- | --- |
| x86_64 | `cargo xtask starry test qemu --target x86_64-unknown-none -c syscall` | 通过 |
| aarch64 | `cargo xtask starry test qemu --target aarch64-unknown-none-softfloat -c syscall` | 通过 |
| riscv64 | `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall` | 通过 |
| loongarch64 | `cargo xtask starry test qemu --target loongarch64-unknown-none-softfloat -c syscall` | 通过 |
