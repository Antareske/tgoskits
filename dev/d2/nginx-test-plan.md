# nginx on StarryOS 测试方案

## 目标

系统化运行 nginx，找出阻碍 nginx 正常工作的内核、libc、文件系统、网络栈和进程语义问题。该方案不以一次性跑通 nginx 为终点，而是建立一个可迭代流程：真实 nginx 场景暴露问题，随后把每个问题沉淀成源码级最小复现测例，避免后续回归。

## 基本原则

- 不先追求完整压力测试，先保证 nginx 的启动、监听、请求处理、退出、日志和配置重载这些核心闭环。
- 每轮只扩大一个维度，例如先单 worker，再多 worker；先短连接，再 keep-alive；先小文件，再大文件和 range。
- 每个阻塞问题都记录为“nginx 场景 + 失败日志 + 涉及 syscall/flag/socket option + 最小 C 复现”。
- 对 StarryOS 已经有源码级测例的位置，优先补到 `test-suit/starryos/normal/qemu-smp1/bugfix/` 或 `syscall/`，不要只保留手工 nginx 验证。
- 完整 nginx smoke 可以作为验收，但不替代源码级 bugfix 测例。

## 测试准备

### nginx 配置集

准备一组最小配置，建议放在 guest 内 `/tmp/nginx-tests/conf/`。

1. `single-worker.conf`
   - `daemon off;`
   - `master_process off;`
   - `worker_processes 1;`
   - `error_log /tmp/nginx-error.log debug;`
   - `pid /tmp/nginx.pid;`
   - listen `127.0.0.1:8080`
   - root `/tmp/nginx-tests/www`

2. `master-one-worker.conf`
   - `daemon off;`
   - `master_process on;`
   - `worker_processes 1;`
   - 覆盖 master/worker channel、signal、waitpid、pid 文件语义。

3. `master-two-workers.conf`
   - `worker_processes 2;`
   - 覆盖 accept 竞争、worker 间 channel、进程回收和共享 listen fd。

4. `features-static.conf`
   - 开启/关闭 `sendfile`
   - 配置 `keepalive_timeout`
   - 配置 `client_max_body_size`
   - 配置 access/error log

### 静态文件集

准备 `/tmp/nginx-tests/www/`：

- `index.html`：小文件，内容固定。
- `empty.txt`：0 字节文件。
- `small.txt`：几十字节。
- `large.bin`：至少 1 MiB，用于 `sendfile`、分段读取、range。
- `dir/index.html`：目录索引解析。
- `not-readable.txt`：如 StarryOS 支持权限，用于 403。
- 缺失路径 `/missing.txt`：用于 404。

### 客户端工具

优先使用 guest 内可用工具，减少 host/QEMU 网络转发变量：

- `curl` 或 `wget`
- `nc` 或 `busybox nc`
- 简单 shell 循环

若需要 host 访问 nginx，再使用 QEMU user net hostfwd，例如把 host `18080` 转发到 guest `8080`。host 访问适合压力和长连接调试，但首轮不依赖它。

## 阶段 0：启动前探测

目的：确认 nginx 运行前的基础环境，不把缺文件、路径、权限误判为内核问题。

检查项：

- `nginx -v`、`nginx -V` 是否可执行。
- `nginx -t -c /tmp/nginx-tests/conf/single-worker.conf`。
- `/tmp` 可写，能创建 pid、log、client body temp。
- `/dev/null`、`/dev/zero`、`/proc/self/fd`、`/proc/self/stat`、`/proc/meminfo` 的访问情况。
- `getrlimit(RLIMIT_NOFILE)`、`setrlimit` 行为是否影响 nginx 启动。

输出记录：

- nginx version/build options。
- `nginx -t` stdout/stderr。
- error log。
- 若失败，优先归类到文件系统/proc/权限/rlimit。

## 阶段 1：最小启动闭环

### 1.1 单进程无 master

命令形态：

```sh
nginx -c /tmp/nginx-tests/conf/single-worker.conf -p /tmp/nginx-tests/
```

期望：

- 进程不崩溃。
- 监听 `127.0.0.1:8080` 成功。
- pid/log 文件按配置生成。
- `curl http://127.0.0.1:8080/` 返回 200。
- `nginx -s quit` 或直接 SIGTERM 后能退出。

重点暴露：

- `socket/bind/listen/getsockname`
- `setsockopt(SO_REUSEADDR, TCP_NODELAY, SO_KEEPALIVE)`
- `fcntl(F_SETFL O_NONBLOCK)`
- `epoll_create/epoll_ctl/epoll_wait`
- `openat/stat/read/write/close`
- `clock_gettime/gettimeofday`

### 1.2 master + 1 worker

期望：

- master fork/clone worker 成功。
- worker channel 初始化成功。
- 请求能被 worker 处理。
- SIGTERM/SIGQUIT 能让 master 和 worker 有序退出。
- worker 退出后 master `waitpid` 能回收。

重点暴露：

- `clone/fork/execve` 相关语义。
- `socketpair(AF_UNIX, SOCK_STREAM)`。
- `FIONBIO/FIOASYNC/F_SETOWN/F_SETFD`。
- `sigaction/sigprocmask/sigsuspend/rt_sigreturn`。
- `wait4`、`kill`、`getppid/getpid`。

### 1.3 master + 2 workers

期望：

- 两个 worker 都能启动。
- 多次请求不会只卡在单 worker 或 accept 失败。
- 退出时两个 worker 都被回收。

重点暴露：

- 共享 listen fd 的 `accept/accept4`。
- 多进程文件描述符继承和 close-on-exec。
- 多 worker 下 epoll wakeup、公平性、惊群相关问题。
- pid/ppid 和 zombie 行为。

## 阶段 2：HTTP 基本语义

每个用例记录 HTTP 状态码、响应头、响应体摘要和 nginx error log。

1. GET `/`
   - 期望 200，返回 `index.html`。

2. GET `/small.txt`
   - 期望 200，`Content-Length` 正确。

3. GET `/empty.txt`
   - 期望 200，`Content-Length: 0`。

4. GET `/missing.txt`
   - 期望 404，不崩溃。

5. HEAD `/small.txt`
   - 期望只有头，无响应体。

6. GET `/dir/`
   - 期望返回 `/dir/index.html`。

7. GET `/dir`
   - 期望 301/302 到 `/dir/` 或符合 nginx 行为。

8. 非法方法 `BAD / HTTP/1.1`
   - 期望 400/405，不导致连接状态异常。

重点暴露：

- 路径解析、`stat/open/read`。
- `writev/send/sendfile`。
- socket 半关闭、短连接关闭。
- header 解析和错误路径。

## 阶段 3：连接模型

### 3.1 短连接

循环请求 100 次：

```sh
for i in $(seq 1 100); do curl -fsS http://127.0.0.1:8080/small.txt >/dev/null || exit 1; done
```

期望：

- 无请求失败。
- nginx 不泄漏 fd。
- 无 worker 崩溃。

### 3.2 keep-alive

用 `curl` 或 `nc` 在同一 TCP 连接内连续发送多个 HTTP/1.1 请求。

期望：

- 同连接多请求都成功。
- `Connection: keep-alive` 和 `Connection: close` 行为正确。
- idle timeout 后连接关闭，不挂死 epoll。

重点暴露：

- epoll level/edge 行为。
- 非阻塞 read/write 返回 `EAGAIN`。
- TCP close、FIN、RST 状态处理。
- timer/timeout。

### 3.3 慢请求头

用 `nc` 分段发送请求头，每段间隔 sleep。

期望：

- 未超时时可继续解析。
- 超时后连接关闭。
- 不阻塞 worker 处理其他连接。

重点暴露：

- timer。
- 非阻塞 socket。
- epoll timeout。

## 阶段 4：文件发送路径

### 4.1 sendfile off

配置 `sendfile off;`，请求 `large.bin`。

期望：

- body 长度和校验和正确。
- 多次请求稳定。

重点暴露：

- `read + writev/write` 路径。
- 大 buffer 循环。

### 4.2 sendfile on

配置 `sendfile on;`，请求 `large.bin`。

期望：

- 与 sendfile off 内容一致。
- 不出现短写处理错误。

重点暴露：

- `sendfile` offset 更新。
- socket 非阻塞短写。
- 文件页缓存、mmap/read fallback。

### 4.3 Range 请求

请求：

```sh
curl -H 'Range: bytes=0-15' http://127.0.0.1:8080/large.bin
curl -H 'Range: bytes=100-199' http://127.0.0.1:8080/large.bin
curl -H 'Range: bytes=-64' http://127.0.0.1:8080/large.bin
```

期望：

- 206。
- `Content-Range` 正确。
- body 长度正确。

重点暴露：

- `lseek/pread/sendfile` offset。
- 文件长度和 stat。

## 阶段 5：请求体和临时文件

即使只测静态 nginx，也要覆盖 request body，因为 nginx 会读取并可能写 client body temp。

1. 小 POST：
   - `curl -X POST --data 'abc' http://127.0.0.1:8080/`
   - 期望 nginx 返回静态场景下的合理错误或 405/404，不崩溃。

2. 大 POST：
   - 发送超过内存 buffer 的 body。
   - 观察是否创建 client body temp file。

3. 超过 `client_max_body_size`：
   - 期望 413。

重点暴露：

- socket read request body。
- 临时文件 `open/unlink/write/read`。
- `rename/unlink`。
- 磁盘空间和错误返回。

## 阶段 6：日志和文件系统语义

检查：

- access log 每次请求写入一行。
- error log debug 信息完整。
- log reopen：发送 `USR1` 后能重新打开日志。
- pid 文件创建和删除。
- config 中相对路径基于 `-p` prefix 解析。

重点暴露：

- `openat(O_APPEND)`。
- `write` append 原子性。
- `fstat/stat`。
- `rename/unlink`。
- signal 驱动 log reopen。

## 阶段 7：信号和生命周期

覆盖 nginx 常见控制命令：

1. `nginx -s stop`
   - 快速退出。

2. `nginx -s quit`
   - 优雅退出，已有连接处理完。

3. `nginx -s reload`
   - master 重新加载配置。
   - 新 worker 启动，旧 worker 退出。

4. `nginx -s reopen`
   - 日志重新打开。

5. worker 人为 kill
   - master 能感知并按 nginx 策略重启或退出。

重点暴露：

- `kill` 权限和目标选择。
- `SIGTERM/SIGQUIT/SIGHUP/SIGUSR1/SIGCHLD`。
- `wait4`。
- signal mask 和 pending 信号。
- 多进程 fd 生命周期。

## 压力测试（独立管理）

并发与压力测试不再作为阶段测试的一部分，统一从本计划剥离，单独在
`apps/starry/nginx/stress/` 管理。

当前保留的压力项清单：

1. 并发 2，100 请求。
2. 并发 8，1000 请求。
3. 并发 32，5000 请求。
4. keep-alive 并发连接。
5. 大文件并发下载。
6. 混合 200/404/range/large。

阶段测试文档仅保留功能正确性与语义定位内容。

## 阶段 9：配置特性扩展

按功能逐项打开，不要一次启用完整默认配置。

1. `gzip off/on`
   - 如果 nginx 编译支持 gzip，覆盖 zlib、mmap/brk/mremap、CPU 时间。

2. `autoindex on`
   - 覆盖 `getdents64`、目录 stat、排序内存。

3. `try_files`
   - 覆盖路径查找和 fallback。

4. `error_page`
   - 覆盖内部重定向。

5. `alias`
   - 覆盖路径拼接边界。

6. IPv6 listen
   - 若 StarryOS 目标支持 IPv6，再测试 `[::1]:8080`。

7. Unix domain socket listen
   - 如果 nginx 配置支持并且 StarryOS AF_UNIX stream 已足够稳定，再测试 `listen unix:/tmp/nginx.sock`。

## 阶段 10：问题归档模板

每发现一个阻塞点，按以下格式记录：

```text
标题：
nginx 阶段：
配置文件：
触发命令：
期望行为：
实际行为：
nginx error log：
StarryOS log：
疑似 syscall/语义：
Linux 对照结果：
最小复现测例路径：
修复 PR/commit：
回归验证命令：
```

## 源码级回归测例沉淀策略

建议按 nginx 暴露的语义归类，而不是为每个 nginx 现象写一个大而全的测例。

优先补源码级测例的类别：

- `bug-nginx-fioasync` 类：nginx channel、fcntl/ioctl、socketpair。
- `bug-nginx-signal-lifecycle`：master/worker signal、wait4、SIGCHLD。
- `bug-nginx-epoll-accept`：listen fd、nonblocking accept、epoll readiness。
- `bug-nginx-sendfile-range`：sendfile、offset、range。
- `bug-nginx-keepalive-close`：keep-alive、多请求、FIN/RST。
- `bug-nginx-log-reopen`：O_APPEND、USR1、fd reopen。
- `bug-nginx-temp-file`：request body temp、unlink、rename。
- `bug-nginx-proc-fd`：`/proc/self/fd`、pid、stat。

每个源码级测例应满足：

- 未修复内核上稳定失败。
- 修复后输出 `TEST PASSED`。
- 只覆盖一个明确语义，不依赖完整 nginx。
- 接入 `bugfix` 或 `syscall` grouped case 后，可用 `--subcase` 单独运行。

## 推荐执行顺序

1. 阶段 0：`nginx -t` 和环境探测。
2. 阶段 1.1：单进程无 master，拿到第一个 HTTP 200。
3. 阶段 2：基础 HTTP 语义。
4. 阶段 1.2：master + 1 worker。
5. 阶段 7：stop/quit/reload/reopen。
6. 阶段 1.3：master + 2 workers。
7. 阶段 3：短连接、keep-alive、慢请求。
8. 阶段 4：大文件、sendfile、range。
9. 阶段 5：请求体和临时文件。
10. 阶段 9：逐项打开 nginx 配置特性。

## 第一轮验收标准

第一轮不要求压力性能，只要求功能闭环：

- `nginx -t` 成功。
- `master_process off` 下 GET `/` 成功。
- `master_process on; worker_processes 1` 下 GET `/` 成功。
- 404、HEAD、keep-alive 两连请求成功。
- `nginx -s quit` 能优雅退出，无 zombie。
- access log 和 error log 可写。
- 已知失败点均有最小复现计划或已落地源码级测例。

## 第二轮验收标准

- `worker_processes 2` 可稳定服务。
- 100 次短连接请求无失败。
- keep-alive 多请求无失败。
- `sendfile off/on` 大文件内容一致。
- range 请求返回 206 且内容正确。
- `reload/reopen` 不破坏后续请求。
- 发现的阻塞点均已转化为 `bugfix` 或 `syscall` 下可单独运行的源码级测例。

## 不建议首轮投入的内容

- TLS/HTTPS：除非 nginx 已编译 OpenSSL 且基础 HTTP 完整通过。
- 反向代理 upstream：会引入第二服务端，先等静态 HTTP 稳定。
- 高并发性能指标：先查正确性，后查性能。
- 全默认 nginx 配置：默认配置会同时触发大量模块，不利于定位。
