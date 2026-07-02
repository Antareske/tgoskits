# 文件同步 syscall 最终总结

## 完成内容

本轮围绕 `www/my-job.md` 中的 5 个文件同步 syscall 完成了以下工作：

1. 依据 `www/syscall-man` 生成 Linux 语义驱动的 C 测例
2. 将测例集成到 `test-suit/starryos/normal/qemu-smp1/syscall`
3. 在宿主 Linux 上分别用 `gcc` 和 `musl-gcc` 构建并运行，确认测例本身与 Linux 行为对齐
4. 执行 `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall` 获取 StarryOS 修复前行为
5. 根据失败点修复 StarryOS 内核实现
6. 执行 `cargo fmt`
7. 执行 `cargo xtask clippy --package starry-kernel`
8. 重新执行 riscv64 `syscall` 测试，确认通过

## 新增测例

新增 5 个子测例：

- `test-fsync`
- `test-fdatasync`
- `test-sync`
- `test-syncfs`
- `test-sync-file-range`

并已加入以下 CI 运行配置：

- `test-suit/starryos/normal/qemu-smp1/syscall/qemu-riscv64.toml`
- `test-suit/starryos/normal/qemu-smp1/syscall/qemu-aarch64.toml`
- `test-suit/starryos/normal/qemu-smp1/syscall/qemu-x86_64.toml`
- `test-suit/starryos/normal/qemu-smp1/syscall/qemu-loongarch64.toml`

## 宿主 Linux 基线验证结果

两套 libc 下均通过：

- `gcc`: 5/5 通过
- `musl-gcc`: 5/5 通过

这一步确认了测例不是“为 StarryOS 定制的期望”，而是真正贴近 Linux 当前行为。

## StarryOS 修复前问题

修复前只有两个 syscall 与 Linux 语义不一致：

### `sync_file_range`

- 合法文件 fd + 非法 `flags` 时，StarryOS 返回 `0`，Linux 返回 `EINVAL`
- pipe fd 时，StarryOS 返回 `EINVAL`，Linux 返回 `ESPIPE`

### `syncfs`

- 目录 fd 时，StarryOS 返回 `EISDIR`
- pipe fd 时，StarryOS 返回 `EINVAL`
- Linux 对这两种合法已打开 fd 都返回 `0`

## 内核修复内容

### `sys_sync_file_range`

修改文件：`os/StarryOS/kernel/src/syscall/fs/io.rs`

修复后的逻辑顺序：

1. `get_file_like(fd)?` 先校验 fd 是否有效，保证无效/关闭 fd 返回 `EBADF`
2. 检查 `flags & !0x7`，非法 bit 返回 `EINVAL`
3. 仅允许 `File` 或 `Directory`；其他类型如 pipe 返回 `ESPIPE`
4. 继续保持当前 no-op 语义，合法调用返回 `0`

### `sys_syncfs`

修改文件：`os/StarryOS/kernel/src/syscall/fs/ctl.rs`

修复后的逻辑：

1. `get_file_like(fd)?` 统一校验 fd 合法性
2. `File` fd 调用其所属 `filesystem().flush()`
3. `Directory` fd 调用目录所属 `filesystem().flush()`
4. 其他合法 `FileLike`（如 pipe）直接返回 `0`

这样既保留了对真实文件系统对象的 flush 行为，也与 Linux “任意合法已打开 fd 都可调用 syncfs”的行为对齐。

## 最终验证结果

### 格式化

```bash
cargo fmt
```

通过。

### Clippy

```bash
cargo xtask clippy --package starry-kernel
```

通过。

### StarryOS syscall 测试

```bash
cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall
```

结果：

- `ok: syscall`
- `PASS syscall (88.45s)`
- `all starry normal qemu tests passed`

## 本轮结论

本轮文件同步 syscall 工作已经完成闭环：

- 测例来自 Linux 语义
- 测例先在宿主 Linux 上验证
- StarryOS 修复前确实能稳定暴露 bug
- 内核修复后测例在 StarryOS 上通过
- 修改后的 `starry-kernel` 已通过格式化与 clippy 校验
