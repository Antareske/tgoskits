# 提交简要说明

- 提交：`470f8dbca`
- 标题：`test(starry,epoll): add standalone signal-interrupt EINTR regression case`

本次提交新增 Starry normal 独立 case：`test-signal-interrupt-eintr`，用于直接固定信号打断阻塞 syscall 的 ABI 语义。

- 子进程阻塞在 `poll(..., -1)`；
- 父进程发送未屏蔽 `SIGUSR1`，子进程已安装 handler；
- 断言阻塞 syscall 返回 `-1` 且 `errno == EINTR`。

该 case 已补齐四个架构配置：`riscv64` / `aarch64` / `x86_64` / `loongarch64`。

测试结果：四个架构的 test-signal-interrupt-eintr 测试均通过（`riscv64` / `aarch64` / `x86_64` / `loongarch64`）。
