# StarryOS nginx 问题记录

## ISSUE-001: socket `ioctl(FIONREAD)` 语义导致 nginx 大 POST empty reply

状态：已复现，未修复。

触发场景：

- nginx `client_max_body_size 4k; client_body_buffer_size 1k;`
- 客户端向静态 location POST 8KiB body。

期望行为：

- nginx 返回 HTTP 413，并保持可诊断的 HTTP 响应。

实际行为：

- `curl` 报 `curl: (52) Empty reply from server`。
- 响应头文件为空。
- nginx access log 没有该请求的完整 HTTP 状态行。

关键日志：

```text
StarryOS: Unsupported ioctl command: 21531 for fd: 8
nginx: ioctl(FIONREAD) failed (25: Not a tty) while waiting for request
```

疑似根因：

- nginx 在 socket fd 上调用 `ioctl(FIONREAD)` 判断待读 request body 字节数。
- StarryOS 当前没有为 socket fd 支持该 ioctl，或把它错误地走到了 tty/通用 unsupported 路径。

建议最小复现：

- C 测例创建 TCP loopback 连接。
- 客户端发送若干字节但不关闭。
- 服务端 accepted socket 调用 `ioctl(fd, FIONREAD, &n)`。
- Linux 期望：返回 0，`n` 为当前可读字节数。
- StarryOS 当前疑似：返回失败，errno 为 ENOTTY 或 unsupported。

回归位置建议：

- `test-suit/starryos/normal/qemu-smp1/bugfix/bug-nginx-fionread-socket/`

## ISSUE-002: nginx 启动路径反复触发 `io_setup` ENOSYS

状态：已观察，当前不阻塞基础 nginx smoke。

触发场景：

- Alpine nginx 1.28.3 编译参数含 `--with-file-aio`。
- 单进程 nginx、master worker、reload 新 worker、sendfile 实例启动均会触发。

关键日志：

```text
StarryOS: Unimplemented syscall: io_setup
nginx: io_setup() failed (38: Function not implemented)
```

影响判断：

- 当前 nginx 能 fallback 并继续处理静态 HTTP。
- 后续启用 file AIO 或复杂文件发送路径时可能成为功能缺口。

建议最小复现：

- C 测例直接调用 `io_setup`，确认返回 ENOSYS 是否与 Linux/应用 fallback 预期一致。
- 如项目目标是完整支持 nginx file AIO，则需要进一步覆盖 `io_setup/io_destroy/io_submit/io_getevents`。
