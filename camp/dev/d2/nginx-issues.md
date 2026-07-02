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

## ISSUE-003: `master_process on; worker_processes 2` 在 StarryOS 上存在稳定阻塞/不稳定

状态：已复现（通过超时与 known-issue 旁路），未修复。

触发场景：

- nginx phase1 生命周期测试，`master_process on; worker_processes 2;`。
- 启动成功后进行短轮询请求与退出控制。

实际行为：

- 在 StarryOS（riscv64 qemu-smp1）中，2 worker 场景会出现阻塞或请求不稳定，需靠短超时与强制清理兜底。
- 同样流程在宿主机 Linux 对照中稳定通过（2 worker、8 次请求、`nginx -s quit`、无残留 worker）。

影响判断：

- 阶段 1.3（多 worker）当前不宜作为严格 pass gate，只能作为 known-issue 探针持续观察。
- 需要继续下钻 accept 竞争、worker channel、信号回收路径。

建议最小复现：

- 进程模型：master + 2 workers 共享 listen fd。
- 循环：短超时 accept/read/write 请求 + `SIGQUIT` 退出。
- 观测：worker 存活数、请求失败点、退出后残留 worker/zombie。

## ISSUE-004: phase2 非法方法原始 TCP 请求路径不稳定（`BAD / HTTP/1.1`）

状态：已复现（known-issue 旁路），未修复。

触发场景：

- nginx phase2 HTTP 基础语义测试，用 `nc` 发送原始请求：`BAD / HTTP/1.1`。

期望行为：

- 返回 `400` 或 `405`，且连接可控关闭。

实际行为：

- StarryOS 中该路径出现 `punt!` 与会话被 `Terminated` 的现象，导致该子项不稳定。
- 宿主机 Linux 对照中同请求稳定返回 `400/405` 路径（本轮实测匹配成功）。
- 2026-05-28 新定位：同一实例下 `curl -X BAD http://127.0.0.1:8080/` 可稳定返回 `405`；但 `nc` 原始报文 `BAD / HTTP/1.1` 连续 5 次均为 `<empty>`，并伴随 `punt!` + `Terminated`。
- 进一步最小单元矩阵（`nginx-2-0-bad-method-matrix.sh`）显示：
  - `netcat-openbsd` 提供的 `nc`：`BAD` 稳定返回 `HTTP/1.1 405 Not Allowed`，`GET` 稳定 `200`。
  - `busybox nc`：`BAD` 与 `GET` 均可能 `<empty>`。
  - 说明问题不在 nginx BAD method 语义，也不是内核通用 socket 路径；主要收敛到 `busybox nc` 行为差异/兼容性。
- nginx `error.log` 末行显示 `client ... closed keepalive connection`，未见 nginx 崩溃。

影响判断：

- 阶段 2 其余 `GET /small.txt`、`GET /empty.txt`、`GET /dir/`、`GET /dir` 在 StarryOS 可通过。
- 非法方法子项需先保留 known-issue 探针，不作为阻塞 gate。
- 当前定位：`busybox nc` 在该场景存在行为不稳定，不应作为阶段 2 非法方法语义的唯一判定工具。
