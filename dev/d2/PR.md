我先前的提交：fix(starry,nginx): multi-worker signal interruption and EPOLLEXCLUSIVE handling 的 PR 反馈如下：

这轮需要 request changes：实现方向没问题，但这两个改动都属于 StarryOS syscall/信号 ABI 行为，需要补 normal test-suit 回归测试后再合。

需要补的测试见 inline：
- `EPOLLEXCLUSIVE`：直接验证 `epoll_ctl(..., EPOLLIN | EPOLLEXCLUSIVE)` 不再返回 `EINVAL`，并最好保留未知 flag 仍为 `EINVAL` 的负例。
- 信号打断：直接验证阻塞在 `interruptible` syscall 路径上的线程/进程收到可投递信号后返回 `EINTR`，不要只依赖 nginx app 场景。

原因是 nginx multi-worker 是集成验证，能证明场景通过，但不足以固定这两个底层行为。后续如果 epoll flag 校验或 `task.interrupt()` 语义被回退，normal syscall/bugfix case 应该能第一时间失败。


os/StarryOS/kernel/src/file/epoll.rs:41
这里需要补一个 StarryOS normal 测试，直接覆盖 `epoll_ctl(EPOLL_CTL_ADD, ..., EPOLLEXCLUSIVE)` 不再返回 `EINVAL`。当前 PR 只说明 nginx app 场景通过，但这个兼容点属于 syscall/epoll ABI 行为，建议放到现有 `test-suit/starryos/normal/qemu-smp1/syscall` 或 epoll 相关 bugfix case 里。测试只需创建 epoll fd 和 eventfd/pipe/socket fd，用 `EPOLLIN | EPOLLEXCLUSIVE` 调 `epoll_ctl`，期望返回 0；同时保留一个未知 flag 仍返回 `EINVAL` 的负例会更稳。


os/StarryOS/kernel/src/task/signal.rs:528
这个语义变化也需要补一个直接回归测试，不能只依赖 nginx 多 worker。建议加一个 StarryOS normal case：子进程或线程阻塞在可被 future::interruptible 包住的 syscall（例如阻塞 accept4、epoll_wait、poll/select），父进程发送未屏蔽信号并安装 handler，断言阻塞 syscall 返回 -1 且 errno == EINTR。这样能固定本 PR 的核心行为：可投递信号必须通过 task.interrupt() 打断阻塞 syscall；同时可避免以后又退回 wake_task() 时只在 nginx 场景里才暴露。