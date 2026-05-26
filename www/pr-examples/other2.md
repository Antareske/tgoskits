## 背景

我目前在 TGOSKits / StarryOS 中推进 modern fd family 相关 syscall 的适配与测试，主要包括 `memfd_create`、`F_ADD_SEALS` / `F_GET_SEALS`、`pidfd_open`、`pidfd_send_signal`、`pidfd_getfd` 等路径。

相关 PR：[#565](https://github.com/rcore-os/tgoskits/pull/565)

## 已完成进展

- 实现 `memfd_create` 的匿名 tmpfs inode 创建路径，避免暴露为普通路径可见文件。
- 为 memfd 记录 `MemFdMeta`，包含是否允许 sealing 以及当前 seal 标志位。
- 支持 `fcntl(F_GET_SEALS)` / `fcntl(F_ADD_SEALS)` 的基本语义。
- 补充 `F_SEAL_WRITE`、`F_SEAL_GROW`、`F_SEAL_SHRINK` 在以下路径上的检查：
  - `write` / `writev`
  - `pwrite` / `pwritev`
  - `ftruncate`
  - `fallocate`
  - `sendfile`
  - `copy_file_range`
  - `mmap(MAP_SHARED|PROT_WRITE)`
- 根据 review 修复 Linux ABI 差异：
  - 未设置 `MFD_ALLOW_SEALING` 时，memfd 初始 seals 包含 `F_SEAL_SEAL`。
  - 添加 `F_SEAL_WRITE` 时，如果已有 `MAP_SHARED|PROT_WRITE` 映射，则返回 `EBUSY`。
- 新增 `test-modern-fd-family` 测试用例，覆盖 memfd 和 pidfd 的正常路径、错误路径、seal enforcement 以及部分 Linux ABI 边界。

## 当前验证情况

- `cargo fmt` 通过。
- `cargo xtask clippy --package starry-kernel` 通过。
- `cargo xtask starry test qemu --arch x86_64 --test-case syscall` 通过。
- 远端 CI 中 StarryOS / ArceOS / Axvisor 多架构 QEMU 和部分 board 测试通过。

## TODO

- [ ] 根据 review 继续完善 memfd seal 与 Linux ABI 的边界语义。
- [ ] 补充 `mprotect` 场景：已有 `F_SEAL_WRITE` 后，`MAP_SHARED` 映射不应再通过 `mprotect(PROT_WRITE)` 获得可写权限。
- [ ] 继续扩展 `pidfd_getfd` 的跨进程权限、错误码和生命周期测试。
- [ ] 对照 Linux 行为补充更多 fd-family 边界测试，例如 close-on-exec、dup、poll/epoll 相关交互。
- [ ] 在更多架构上复测 grouped syscall case，例如 aarch64 / riscv64 / loongarch64。
- [ ] 跟进 PR review，合并前保持 CI 全绿。