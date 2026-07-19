# 文件同步 syscall 语义测试结果（修复前）

## 工作范围

根据 `www/my-job.md` 的“文件同步”条目，新增并接入以下 5 个 syscall 测例：

- `fsync`
- `fdatasync`
- `sync_file_range`
- `sync`
- `syncfs`

测例位置：`test-suit/starryos/normal/qemu-smp1/syscall/`

## Linux 基线验证

按 `docs/docs/development/starryos.md` 第 7 节要求，先在宿主 Linux 上分别用 `gcc` 和 `musl-gcc` 构建并运行 5 个测例。

结果：

- `gcc` 构建运行：5/5 通过
- `musl-gcc` 构建运行：5/5 通过

确认后的关键 Linux 语义：

- `fsync(file)`、`fsync(O_RDONLY file)`、`fsync(directory)` 返回 `0`
- `fsync(pipe)` 返回 `EINVAL`
- `fdatasync(file)`、`fdatasync(O_RDONLY file)`、`fdatasync(directory)` 返回 `0`
- `fdatasync(pipe)` 返回 `EINVAL`
- `sync()` 无返回值，调用后程序继续执行
- `syncfs(file)`、`syncfs(directory)`、`syncfs(pipe)` 返回 `0`
- `syncfs(-1)`、`syncfs(closed fd)` 返回 `EBADF`
- `sync_file_range(file, ..., 0)` 返回 `0`
- `sync_file_range(file, ..., 0xff)` 返回 `EINVAL`
- `sync_file_range(directory, ..., WRITE)` 返回 `0`
- `sync_file_range(pipe, ..., WRITE)` 返回 `ESPIPE`
- `sync_file_range(pipe, ..., 0xff)` 返回 `EINVAL`
- `sync_file_range(-1, ..., WRITE)` 返回 `EBADF`
- `sync_file_range(-1, ..., 0xff)` 仍返回 `EBADF`

## StarryOS 修复前测试

执行命令：

```bash
cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall
```

结果：`syscall` case 失败，新增 5 个测例中 3 个通过，2 个失败。

### 通过的测例

- `test-fsync`
- `test-fdatasync`
- `test-sync`

### 失败的测例

#### `test-sync-file-range`

失败点 1：合法文件 fd + 非法 flags 未返回 `EINVAL`

```text
FAIL ... 合法文件 fd + 非法 flags 返回 EINVAL | expected errno=22 got ret=0 errno=0
```

失败点 2：pipe fd 未返回 `ESPIPE`

```text
FAIL ... pipe 读端 sync_file_range 返回 ESPIPE | expected errno=29 got ret=-1 errno=22
```

说明：当前 StarryOS 会忽略非法 `flags`，并把 pipe 的类型错误映射成 `EINVAL`。

#### `test-syncfs`

失败点 1：目录 fd 未返回成功

```text
FAIL ... 目录 fd 上 syncfs 返回 0 | expected=0 got=-1 | errno=21 (Is a directory)
```

失败点 2：pipe fd 未返回成功

```text
FAIL ... pipe 读端 syncfs 返回 0 | expected=0 got=-1 | errno=22 (Invalid argument)
```

说明：当前 StarryOS `syncfs` 只接受 `File::from_fd(fd)`，不能接受目录和 pipe 这类同样合法的已打开 fd。

## 当前结论

修复前 StarryOS 与 Linux 的主要语义偏差集中在两个 syscall：

- `sync_file_range`
  - 缺少非法 `flags` 检查
  - pipe fd 错误码应为 `ESPIPE`
- `syncfs`
  - 应接受任意合法已打开 fd，而不只是 regular file fd
