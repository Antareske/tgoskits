# 响应 `EPOLLEXCLUSIVE` request change

## 1. 内核修改

### `os/StarryOS/kernel/src/syscall/io_mpx/epoll.rs`
- `EPOLL_CTL_ADD` 复用 `parse_event()`，避免 ADD/MOD 各自重复解析用户态 `epoll_event`。
- 略改动 `parse_event()`，新增返回原始标志，供 ADD 分支进行判断。
- `ADD` 分支保留对 `EPOLLEXCLUSIVE` 的 Linux ABI 校验：只允许合法组合，且 target 不能是 epoll instance。
- `MOD` 分支继续拒绝 `EPOLLEXCLUSIVE`，并与已 exclusive 的旧 entry 一起保持 `EINVAL`。
- `EPOLLWAKEUP` 目前不做特判放行，原因是 StarryOS 先前并未感知该标志，也没有 wake 语义实现；因此 `EPOLLWAKEUP | EPOLLEXCLUSIVE` 仍保持为非法，和 Starry 既有行为一致，但与 Linux 语义不一致。我们已在 Linux 环境复测，`EPOLLWAKEUP | EPOLLEXCLUSIVE` 实际是可被接受的。
- 若后续确有兼容需要，可以临时在 `parse_event()` 中单独放行 `EPOLLWAKEUP`；但由于 StarryOS 目前没有 wake 实现，这样做只会是 no-op。也就是说，一旦未来补齐 wake 语义，必须重新审视这条路径，避免误以为当前放行就等价于 Linux 的完整行为。若有对齐需要，建议另开 PR。

### `os/StarryOS/kernel/src/file/epoll.rs`
- `EpollInterest` 新增 `exclusive` 状态，用于记录 entry 是否由 `EPOLLEXCLUSIVE` 添加。
- `modify()` 在替换旧 entry 前检查 `old.is_exclusive()`，确保 exclusive entry 后续 `MOD` 仍然返回 `EINVAL`。
- 这里没有引入 wake 相关状态，因为本次修复只针对 `EPOLLEXCLUSIVE` 的 ABI 兼容，不扩展 wake 语义。

## 2. 测例修改

### `test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-exclusive/c/src/main.c`
- 保留正例：`EPOLLIN | EPOLLEXCLUSIVE` 的 `EPOLL_CTL_ADD` 必须成功。
- 补齐负例：`EPOLL_CTL_MOD` 携带 `EPOLLEXCLUSIVE`、exclusive entry 后再 `MOD`、`EPOLLONESHOT / EPOLLRDHUP / EPOLLPRI` 搭配 `EPOLLEXCLUSIVE`、epoll target 携带 `EPOLLEXCLUSIVE`，都应返回 `EINVAL`。
- 新增 `EPOLLWAKEUP | EPOLLEXCLUSIVE` 负例，并在注释中说明 StarryOS 目前不放行 `EPOLLWAKEUP`，因此该组合仍应失败。

## 已通过验证

- `cargo fmt`
- `cargo xtask clippy --package starry-kernel`
- `cargo xtask starry test qemu --arch x86_64 -c syscall`
- `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf`
