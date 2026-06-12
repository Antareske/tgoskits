# 提交简要说明

- 提交：`39a11fd67`
- 标题：`test(starry,epoll): add EPOLLEXCLUSIVE syscall regression case`

本次提交在 Starry `syscall` 测试集中新增 `test-epoll-exclusive` 用例，直接覆盖 `epoll_ctl` 的 `EPOLLEXCLUSIVE` ABI 语义。

- 正例：验证 `epoll_ctl(..., EPOLLIN | EPOLLEXCLUSIVE)` 返回 `0`。
- 反例：验证注入当前 ABI 下未知事件位时返回 `EINVAL`。

该用例已集成在 `test-suit/starryos/normal/qemu-smp1/syscall/`，会随 `-c syscall` 自动执行。

测试结果：四个架构的 syscall 测试均通过（`riscv64` / `aarch64` / `x86_64` / `loongarch64`）。
