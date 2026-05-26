# StarryOS nginx 自动化测试记录

## 2026-05-22 初始记录

- 目标：自动暴露 nginx 在 StarryOS 上启动、监听、基础 HTTP 请求和退出链路中的阻塞点；本轮只记录问题，不做源码修复。
- 环境：WSL + Docker Compose `tgoskits` 容器；所有 StarryOS/QEMU 命令通过 `docker compose exec -T tgoskits ...` 执行。
- 基础链路：`cargo xtask starry test qemu --arch x86_64 -g normal -c smoke` 已通过，但 QEMU run 约 442s，总耗时约 1072s。
- 新增测试入口：`test-suit/starryos/normal/qemu-smp1/nginx-smoke/`，用于第一轮 nginx smoke。

### 待运行命令

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke
```

### 待记录结果

- `apk update` / `apk add nginx curl` 是否成功。
- `nginx -t` 是否成功。
- `master_process off` + `daemon off` 下 nginx 是否能启动并监听 `127.0.0.1:8080`。
- GET `/`、404、HEAD、keep-alive、日志写入、退出是否通过。

## 2026-05-22 第一轮：`nginx-smoke`

命令：

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke
```

结果：PASS。

关键信息：

- QEMU run 约 521s；case 总耗时约 565s；命令总耗时约 1222s。
- `apk update` 成功，Alpine 源为 `mirrors.cernet.edu.cn/alpine/v3.23`。
- `apk add nginx curl` 成功，安装后显示 `OK: 250.1 MiB in 68 packages`。
- nginx 版本：`nginx/1.28.3`，带 OpenSSL 3.5.6 和较多动态模块。
- `nginx -t -c /tmp/nginx-tests/conf/single-worker.conf` 成功。
- `master_process off` + `daemon off` 下 nginx 能启动并服务 `127.0.0.1:8080`。
- GET `/`、GET 缺失路径返回 404、HEAD `/small.txt`、同连接 keep-alive 两个请求、access/error log 写入、TERM 退出均通过。

暴露/待跟进：

- nginx 启动时 StarryOS 打印 `Unimplemented syscall: io_setup (tid=23)`。当前未阻塞基础静态 HTTP smoke，但 nginx 编译启用了 `--with-file-aio`，后续 file AIO 或相关配置可能受影响，需要作为独立最小复现候选记录。

## 2026-05-22 第二轮：扩展 `nginx-smoke` master/worker

命令：

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke
```

结果：PASS。

新增覆盖：

- `master_process on; worker_processes 1;` 配置通过 `nginx -t`。
- master + 1 worker 能启动并服务 `127.0.0.1:8081`。
- master 模式下 GET `/` 成功。
- `nginx -s reload` 后继续请求 `/small.txt` 成功。
- `nginx -s quit` 能让 master 退出。

重复暴露：

- `Unimplemented syscall: io_setup` 出现 3 次：
  - 单进程 nginx 启动：`tid=24`。
  - master/worker 启动 worker：`tid=47`。
  - reload 后新 worker：`tid=53`。
- 该缺口目前不阻塞基础 HTTP、master/worker、reload、quit，但很可能影响 nginx `file_aio` 或其他 AIO 路径。

## 2026-05-22 第三轮：sendfile/range/POST

命令：

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke
```

结果：FAIL，失败点为“超限 POST 应返回 413”。

已通过场景：

- 1MiB `large.bin` 生成成功。
- 单进程 nginx、master + 1 worker、reload、quit 仍通过。
- `sendfile on` 配置通过 `nginx -t` 并能启动。
- `GET /large.bin` 返回 200，body 长度 1,048,576，`cmp` 与源文件一致。
- `Range: bytes=0-15` 返回 206，`Content-Range: bytes 0-15/1048576`，body 长度 16。
- 小 POST 返回 nginx 静态文件场景下的 405，符合可接受行为。

明确问题 1：超限 POST 未返回 413

- 触发配置：`client_max_body_size 4k; client_body_buffer_size 1k;`
- 触发命令：向 `http://127.0.0.1:8082/` POST 8KiB body。
- 期望：nginx 返回 HTTP 413。
- 实际：`curl: (52) Empty reply from server`，`post-large.headers` 为空。
- StarryOS kernel log：`Unsupported ioctl command: 21531 for fd: 8`。
- nginx error log：`ioctl(FIONREAD) failed (25: Not a tty) while waiting for request, client: 127.0.0.1, server: 127.0.0.1:8082`。
- 影响判断：这会影响 nginx 读取/判断 request body 的路径，尤其是超限 body、client body temp、慢 POST 等场景。
- 最小复现候选：socket fd 上的 `ioctl(FIONREAD)`。Linux 期望应返回 socket receive queue 可读字节数；当前 StarryOS 对该 ioctl 返回了不符合 socket 语义的 ENOTTY/unsupported。

重复问题：`io_setup` 未实现

- 本轮 `io_setup` 仍在单进程 nginx、master worker、reload worker、sendfile 实例启动时出现。
- nginx error log 中表现为 `io_setup() failed (38: Function not implemented)`。
- 当前不阻塞上述 HTTP 静态场景，但仍应后续拆成 AIO 相关最小复现或确认 nginx 是否可接受 ENOSYS fallback。

## 2026-05-22 第四轮：known issue 旁路后继续跑短连接

命令：

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke
```

结果：PASS。

新增/确认：

- ISSUE-001 稳定复现：超限 POST 再次触发 `Unsupported ioctl command: 21531 for fd: 8` 和 `curl: (52) Empty reply from server`。
- 脚本将 ISSUE-001 记录为 known issue 后继续执行。
- `20` 次短连接请求 `/small.txt` 全部成功。
- `sendfile on` 下 1MiB 静态文件、Range、小 POST 仍通过。

阶段性判断：

- nginx 基础可运行，不是完全启动失败。
- 当前最明确、最值得优先修复的问题是 socket `FIONREAD` 语义。
- `io_setup` 未实现是重复出现的功能缺口，但目前 nginx 会 fallback，不阻塞本轮 smoke。
