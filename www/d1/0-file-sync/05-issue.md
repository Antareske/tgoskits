# StarryOS 文件同步 syscall 的 Linux 语义偏差已定位

## 背景

围绕 `fsync`、`fdatasync`、`sync`、`syncfs` 和 `sync_file_range` 已补充一组 Linux 语义驱动的 C 测例，并接入 `test-suit/starryos/normal/qemu-smp1/syscall` 的 grouped syscall 测试。

## 已定位的问题

测例与宿主 Linux 基线对比后，已经定位到两个主要语义偏差：

- `sync_file_range`
  - 合法文件 fd + 非法 `flags` 时，StarryOS 返回 `0`，Linux 返回 `EINVAL`
  - pipe fd 时，StarryOS 返回 `EINVAL`，Linux 返回 `ESPIPE`
- `syncfs`
  - directory fd 时，StarryOS 返回 `EISDIR`
  - pipe fd 时，StarryOS 返回 `EINVAL`
  - Linux 对合法已打开的 file、directory、pipe fd 都返回 `0`

## 预期语义

- `sync_file_range` 应校验非法 `flags`，并对 pipe 等非 file/directory fd 返回 `ESPIPE`
- `syncfs` 应接受任意合法已打开 fd，只有 invalid/closed fd 返回 `EBADF`

## 当前状态

相关问题已经通过新增测例完成定位，并已完成内核修复与回归验证。当前 4 个 QEMU 架构的 `syscall` grouped case 均已通过。

已完成 syscall 验证：cargo xtask starry test qemu --target `<arch>` -c syscall

```bash
cargo xtask starry test qemu --target x86_64-unknown-none -c syscall
cargo xtask starry test qemu --target aarch64-unknown-none-softfloat -c syscall
cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall
cargo xtask starry test qemu --target loongarch64-unknown-none-softfloat -c syscall
```

