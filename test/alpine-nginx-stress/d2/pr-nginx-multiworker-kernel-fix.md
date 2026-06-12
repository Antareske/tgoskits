# PR 说明：修复 StarryOS 上 nginx 多 worker 生命周期阻塞

## Summary

- 修复 `send_signal_to_thread` / `send_signal_to_process` 对可投递信号仅 `wake_task` 的问题，改为 `task.interrupt()`，确保阻塞 syscall 可按 Linux 语义返回 `EINTR`
- 修复 `epoll_ctl` 对 nginx 使用的 `EPOLLEXCLUSIVE` 标志返回 `EINVAL` 的问题，允许该标志通过内核参数校验
- 新增 nginx debug 最小回归：`master + 2 workers` 的启动、请求、`nginx -s quit` 与回收路径（短超时）
- 维护 app 运行链路：修正 nginx app build 配置中的过时 feature，保证 `cargo xtask starry app run -t nginx` 可直接执行

## 背景与问题

在 StarryOS 上复现 nginx `master_process on; worker_processes 2;` 场景时，存在两类问题叠加：

1. worker 启动阶段失败：`epoll_ctl(...)=EINVAL`，导致 worker 直接退出。
2. worker 阻塞阶段无法被信号稳定打断：信号送达后只唤醒任务，不设置 interrupt 标志，阻塞在 `interruptible(...)` 的路径难以及时返回 `EINTR`。

这使 phase1.3 在历史上长期被标记为 known issue 探针路径。

## 改动细节

### 1) 信号打断语义修复（内核）

- 文件：`os/StarryOS/kernel/src/task/signal.rs`
- 改动：
  - `send_signal_to_thread` 中，可投递信号改为 `task.interrupt()`。
  - `send_signal_to_process` 中，选中的目标线程也改为 `task.interrupt()`。
- 目的：让阻塞在 `axtask::future::interruptible(...)` 的 syscall（如 `accept4`）能及时被中断，返回 `EINTR`，与用户态信号处理/退出路径对齐。

### 2) epoll 参数兼容修复（内核）

- 文件：`os/StarryOS/kernel/src/file/epoll.rs`
- 改动：将 `EPOLLEXCLUSIVE` 纳入 `EpollFlags` 合法标志集合。
- 目的：兼容 nginx worker 初始化时对 epoll 的使用，避免 `epoll_ctl` 因未知 flag 返回 `EINVAL`。

### 3) 回归与调试资产（nginx app）

- 新增：`apps/starry/nginx/debug/nginx-multiworker-quit-tests.sh`
- 新增：
  - `apps/starry/nginx/debug/qemu-x86_64-multiworker.toml`
  - `apps/starry/nginx/debug/qemu-riscv64-multiworker.toml`
  - `apps/starry/nginx/debug/qemu-x86_64-phase1.toml`
- 更新：`apps/starry/nginx/prebuild.sh` 注入 debug/phase1 脚本到 rootfs。
- 更新：
  - `apps/starry/nginx/build-x86_64-unknown-none.toml`
  - `apps/starry/nginx/build-riscv64gc-unknown-none-elf.toml`
  移除过时 `ax-driver/pci` feature。

## 验证结果

- debug 最小回归（x86_64）：`NGINX_MW_TEST_PASSED`
- phase1 脚本本体（x86_64）：`NGINX_PHASE1_TEST_PASSED`
- 代码质量检查：`cargo fmt`、`cargo xtask clippy --package starry-kernel` 全部通过

## 风险与影响面

- `task.interrupt()` 只用于“可投递信号”的路径，阻塞信号和 `sigwait` 特殊唤醒逻辑未改动，影响面可控。
- `EPOLLEXCLUSIVE` 作为 flag 兼容项引入，不改变既有 epoll 事件消费主流程。
- nginx app build 配置修正属于运行链路修复，不改变内核 ABI。

## Test plan

- [x] `cargo fmt`
- [x] `cargo xtask clippy --package starry-kernel`
- [x] `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf`
- [x] `cargo xtask starry test qemu --target aarch64-unknown-none-softfloat`
- [x] `cargo xtask starry test qemu --target x86_64-unknown-none`
- [x] `cargo xtask starry app run -t nginx --arch x86_64 --qemu-config apps/starry/nginx/debug/qemu-x86_64-multiworker.toml`
- [x] `cargo xtask starry app run -t nginx --arch x86_64 --qemu-config apps/starry/nginx/debug/qemu-x86_64-phase1.toml`
- [x] `cargo xtask starry app run -t nginx --arch riscv64 --qemu-config apps/starry/nginx/debug/qemu-riscv64-multiworker.toml`
