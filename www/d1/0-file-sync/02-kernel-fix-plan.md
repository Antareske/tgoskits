# StarryOS 文件同步 syscall 内核修复方案

## 修复目标

让 StarryOS 的以下两个 syscall 与 Linux 对齐：

- `sync_file_range`
- `syncfs`

其余 3 个 syscall（`fsync`、`fdatasync`、`sync`）在本轮新增测例下已经通过，无需修改内核逻辑。

## 问题 1：`sync_file_range`

### 当前问题

当前位置：`os/StarryOS/kernel/src/syscall/fs/io.rs`

当前实现：

- 直接使用 `File::from_fd(fd)`
- regular file 或 directory 返回成功
- 其他错误原样上传
- 完全忽略 `flags`

导致两个语义偏差：

1. 合法文件 fd + 非法 `flags` 返回 `0`，应为 `EINVAL`
2. pipe fd 返回 `EINVAL`，应为 `ESPIPE`

### Linux 基线

宿主 Linux 验证表明：

- `sync_file_range(valid file, ..., 0xff)` -> `EINVAL`
- `sync_file_range(pipe, ..., WRITE)` -> `ESPIPE`
- `sync_file_range(pipe, ..., 0xff)` -> `EINVAL`
- `sync_file_range(-1, ..., 0xff)` -> `EBADF`

这说明错误优先级应为：

1. 先校验 fd 是否存在：无效/关闭 fd -> `EBADF`
2. 再校验 `flags`：非法 bit -> `EINVAL`
3. 最后校验 fd 类型：pipe/socket/eventfd 等 -> `ESPIPE`

### 修复方案

把 `sys_sync_file_range()` 改成 3 步：

1. `get_file_like(fd)?`：先完成 fd-table lookup，保证无效 fd 先报 `EBADF`
2. `if flags & !0x7 != 0 { ... }`：只接受低 3 位
3. 用 `downcast_ref::<File>()` / `downcast_ref::<Directory>()` 判断类型，不是二者则返回 `ESPIPE`

保留当前 no-op 语义：

- 仍不实现真实 range writeback
- 对合法调用返回 `0`

## 问题 2：`syncfs`

### 当前问题

当前位置：`os/StarryOS/kernel/src/syscall/fs/ctl.rs`

当前实现：

```rust
let f = crate::file::File::from_fd(fd)?;
f.inner().location().filesystem().flush()?;
```

因此只有 regular file 或 memfd fd 能成功；目录 fd 和 pipe fd 会失败。

### Linux 基线

宿主 Linux 验证表明：

- `syncfs(file)` -> `0`
- `syncfs(directory)` -> `0`
- `syncfs(pipe)` -> `0`
- `syncfs(-1)` / `syncfs(closed fd)` -> `EBADF`

也就是说，Linux `syncfs` 的最小语义要求是：

- 任意合法已打开 fd 都可以调用
- 无效或关闭 fd 才返回 `EBADF`

### 修复方案

把 `sys_syncfs()` 改成：

1. 先 `get_file_like(fd)?`，统一完成合法 fd 校验
2. 如果是 `File`，调用其 `location().filesystem().flush()`
3. 如果是 `Directory`，调用目录 `filesystem().flush()`
4. 其他 `FileLike`（如 `Pipe`、`Socket`、`EventFd`）暂时按 Linux 行为返回 `0`

这样做的原因：

- 对真实文件系统对象仍保留 flush 动作
- 对没有可解析 filesystem handle 的对象，至少满足“合法 fd 返回成功”的 Linux 语义基线

## 预期修复后结果

- `test-sync-file-range` 全部通过
- `test-syncfs` 全部通过
- `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall` 通过
