# Apache on StarryOS 测试方案

## 目标

系统化运行 Apache httpd，借助真实 Web 服务场景暴露并修复 StarryOS 的进程、线程、文件系统、网络栈、信号、定时器和 libc 语义问题。Apache 测试可以复用 nginx 测试的组织经验，但测试内容必须从 Apache 自身行为出发：配置解析、MPM、模块加载、目录访问控制、日志、graceful 生命周期、CGI 和 Apache 特有 HTTP 行为都应作为独立驱动项。

## 与 nginx 经验的对齐

- 采用 `apps/starry/apache` 作为 app 目录，当前按 `runner/`、`smoke/`、`phase/`、`qemu/phase/`、`debug/`、`stress/` 分层组织。
- `smoke/` 只保留全局入口验收，`phase/` 用于功能阶段迭代，`qemu/phase/` 保存手工复测入口，`debug/` 保存单问题探针，`stress/` 单独管理压力测试。
- 每个 StarryOS 测试项执行前，必须先在 Linux Alpine 环境运行同一脚本或同等命令，确认测试本身正确；Linux 结果只作为对照，不代表 StarryOS 通过。
- 每个脚本必须有短超时、watchdog、失败诊断和强制清理，避免 QEMU guest 被 httpd 残留进程污染。
- 阻塞点记录为“Apache 场景 + 失败日志 + 疑似 syscall/语义 + Linux 对照 + 最小源码级复现”。
- Apache phase/stress/debug 不直接接入 `test-suit/starryos`；修复 StarryOS 后再把最小语义复现沉淀到 `test-suit/starryos/normal/qemu-smp1/bugfix/` 或 `syscall/`。

## 不被 nginx 带偏的原则

- 不把 nginx 的 master/worker 阶段照搬为 Apache 阶段。Apache 的核心差异是 MPM：`prefork` 多进程、`worker/event` 多线程加多进程、accept mutex、scoreboard 和 graceful restart。
- 不只测静态文件。Apache 的目录规则、`Options`、`AllowOverride`、`DirectoryIndex`、`Alias`、`mod_autoindex`、`mod_cgi`、`mod_status`、`ErrorDocument` 等模块行为应成为主要测试对象。
- 不默认 Apache 与 nginx 的响应码完全一致。目录重定向、POST 到静态资源、非法方法、Range、权限错误、`.htaccess` 解析都应以 Linux Alpine Apache 对照为准。
- 不提前追求并发性能。第一目标是让 Apache 的核心功能闭环稳定暴露 StarryOS 缺陷。

## 建议目录

```text
apps/starry/apache/
  README.md
  prebuild.sh
  apache-alpine-mirror.sh
  apache-cli-tests.sh
  runner/
  qemu-riscv64.toml
  qemu/
    all/
    debug/
    phase/
      qemu-riscv64-phase20.toml
      qemu-riscv64-phase30.toml
      qemu-riscv64-phase40.toml
      qemu-riscv64-phase50.toml
      qemu-riscv64-phase55.toml
      qemu-riscv64-phase70.toml
      qemu-riscv64-phase80.toml
  smoke/apache-smoke-tests.sh
  phase/apache-2-0-mpm-prefork-tests.sh
  phase/apache-3-0-http-static-tests.sh
  phase/apache-4-0-directory-access-tests.sh
  phase/apache-5-0-log-lifecycle-tests.sh
  phase/apache-5-5-sendfile-range-tests.sh
  phase/apache-7-0-cgi-tests.sh
  phase/apache-8-0-module-feature-tests.sh
  debug/
  stress/
```

## 测试准备

### 包和命令

Alpine 首轮建议安装：

- `apache2`
- `apache2-utils`
- `curl`
- `busybox-extras`
- `coreutils`

常用命令：

- `httpd -v`、`httpd -V`
- `httpd -M` 查看已加载模块
- `httpd -t -f /tmp/apache-tests/conf/httpd.conf`
- `httpd -X -f ...` 单进程调试模式
- `httpd -DFOREGROUND -f ...` 前台运行
- `apachectl -k start|stop|restart|graceful` 或直接发送信号

### 基础目录

guest 内建议使用 `/tmp/apache-tests/`：

```text
/tmp/apache-tests/
  conf/httpd.conf
  htdocs/index.html
  htdocs/small.txt
  htdocs/empty.txt
  htdocs/large.bin
  htdocs/dir/index.html
  htdocs/auto/a.txt
  htdocs/errors/404.html
  cgi-bin/echo.sh
  logs/
  run/
  out/
```

配置应显式指定 `ServerRoot`、`DocumentRoot`、`PidFile`、`ErrorLog`、`CustomLog`、`TypesConfig`、`Listen 127.0.0.1:8080`，避免依赖 Alpine 默认路径。

## 阶段 0：环境和配置解析

目标：先确认 Apache 可执行、模块和基础运行目录，不把打包缺失误判为 StarryOS 内核问题。

检查项：

- `httpd -v`、`httpd -V` 可执行并输出 build options。
- `httpd -M` 可列出 MPM 和模块。
- `httpd -t -f /tmp/apache-tests/conf/httpd.conf` 通过。
- `/tmp`、`logs/`、`run/` 可写。
- `/dev/null`、`/proc/self/fd`、`/proc/self/stat` 可访问。
- `ulimit -n` 和 `getrlimit(RLIMIT_NOFILE)` 不阻塞 httpd 初始化。

重点暴露：

- 动态链接器和共享库加载。
- `openat/stat/access/readlink`。
- `getrlimit/setrlimit`。
- `/proc`、设备节点、当前工作目录和相对路径解析。

## 阶段 1：前台单进程闭环

优先使用 Apache 自带单进程调试模式：

```sh
httpd -X -f /tmp/apache-tests/conf/httpd.conf
```

期望：

- 进程前台启动，不 daemonize。
- 监听 `127.0.0.1:8080`。
- GET `/` 返回 200。
- pid、access log、error log 按配置生成。
- 发送 SIGTERM 后退出，无残留。

重点暴露：

- `socket/bind/listen/accept`。
- `setsockopt(SO_REUSEADDR/TCP_NODELAY)`。
- 非阻塞 fd、poll/epoll/select 路径。
- `stat/open/read/write/writev/close`。
- `clock_gettime/gettimeofday`。

## 阶段 2：MPM 和进程/线程模型

Apache 的 StarryOS 修复重点应围绕 MPM，而不是照搬 nginx 的 worker 阶段。

### 2.1 prefork MPM

期望：

- `StartServers 1`、`MinSpareServers 1`、`MaxRequestWorkers 2` 可启动。
- 子进程能 accept 并处理请求。
- 多个子进程退出后 parent 可 `waitpid/wait4` 回收。
- parent 收到 SIGTERM/SIGINT 后清理子进程。

重点暴露：

- `fork/clone`、fd 继承、close-on-exec。
- `wait4`、SIGCHLD、僵尸进程。
- accept mutex 相关的 `fcntl/flock/pthread mutex` 或文件锁语义。

当前落地脚本：`apps/starry/apache/phase/apache-2-0-mpm-prefork-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase20.toml`。该脚本当前聚焦 prefork 启动、`/server-status?auto` 可读、请求处理和 clean stop；restart/lifecycle 语义移到阶段 6。

### 2.2 worker/event MPM（若 Alpine 构建支持）

期望：

- 单子进程多线程可启动。
- 多连接请求不因线程调度或 futex 问题挂死。
- keep-alive 连接在 event MPM 下能超时关闭。

重点暴露：

- `pthread_create`、TLS、栈映射。
- `futex`、robust list、线程退出。
- `epoll/poll`、timer、非阻塞 socket。

这部分尚未落地到 StarryOS 阶段脚本，保留为后续可选扩展。

## 阶段 3：静态 HTTP 语义

所有响应码和 header 以 Linux Alpine Apache 对照为准。

执行 StarryOS 测试前，先在 Linux Alpine 上执行相同请求矩阵并保存状态码、关键 header、body 摘要和 Apache error log；若 StarryOS 与 Linux 不一致，优先按 Apache/Linux 对照分析 StarryOS 语义偏差。

检查项：

- GET `/` 返回 `index.html`。
- GET `/small.txt` 返回正确 `Content-Length`。
- GET `/empty.txt` 返回 `Content-Length: 0`。
- GET `/missing.txt` 返回 404。
- HEAD `/small.txt` 只有响应头，无 body。
- GET `/dir/` 返回目录内 `index.html`。
- GET `/dir` 返回 Apache 对照一致的目录 slash redirect。
- 非法方法和未知方法返回 Apache 对照一致状态，不导致连接异常。
- HTTP/1.1 keep-alive 同连接两次请求成功。
- `Connection: close` 后服务端正确关闭连接。

重点暴露：

- 路径规范化、`stat/open/read`。
- HTTP parser 错误路径。
- socket 半关闭、FIN/RST。
- header 生成、短写处理。

## 阶段 4：Apache 目录、权限和路径规则

这是 Apache 区别于 nginx 的首轮重点。

检查项：

- `DirectoryIndex index.html` 生效。
- `Options Indexes` + `mod_autoindex` 可列目录。
- 禁用 `Options Indexes` 时目录无 index 返回 Apache 对照状态。
- `Alias /alias/ /tmp/apache-tests/alias-src/` 路径映射正确。
- `ErrorDocument 404 /errors/404.html` 返回自定义错误页。
- `Require all granted` 与 `Require all denied` 行为正确。
- `AllowOverride All` 时 `.htaccess` 被读取并影响访问。
- `Options FollowSymLinks` / 禁用 FollowSymLinks 的差异按 Linux 对照验证。

当前落地脚本：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase40.toml`。该脚本已覆盖 autoindex、禁用 indexes、Alias、ErrorDocument、Require denied、`.htaccess` DirectoryIndex、FollowSymLinks on/off。

重点暴露：

- 多层目录 `stat/access/open`。
- `.htaccess` 逐级查找。
- symlink、`readlink`、权限检查。
- `getdents64`、目录遍历和排序内存分配。

## 阶段 5：文件发送、Range 和缓存相关语义

Apache 配置项不同于 nginx，应显式测试：

- `EnableSendfile Off` 请求 `large.bin`，校验长度和内容。
- `EnableSendfile On` 请求 `large.bin`，校验长度和内容。
- `EnableMMAP Off/On` 如果模块和平台支持，比较稳定性。
- Range `bytes=0-15`、`bytes=100-199`、`bytes=-64` 返回 206 和正确 body。
- `If-Modified-Since`、`ETag`、`Last-Modified` 基本行为。

重点暴露：

- `sendfile` offset 和短写。
- `mmap/munmap/mprotect`。
- `lseek/pread/stat`。
- 时间戳精度和 HTTP date 格式。

当前落地脚本：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase55.toml`。该脚本已覆盖 sendfile off/on、1MiB 大文件一致性、Range 三种形式、以及 `If-Modified-Since` / `ETag` / `Last-Modified`。

## 阶段 6：日志、runtime 文件和 graceful 生命周期

检查项：

- `CustomLog` 每请求新增一行。
- `ErrorLog` 可写，失败时能输出诊断。
- `PidFile` 创建和退出后处理符合 Apache 对照。
- `apachectl -k graceful` 或 SIGUSR1 后继续服务。
- `apachectl -k restart` 或 SIGHUP 后重新加载配置。
- `apachectl -k stop` 或 SIGTERM 后退出。
- log rotate 场景：移动 log 后 graceful/reopen 是否重新打开。

重点暴露：

- `open(O_APPEND)`、append 原子性。
- `rename/unlink`。
- signal mask、pending signal、SIGUSR1/SIGHUP/SIGTERM/SIGCHLD。
- parent/child fd 生命周期。

当前落地脚本：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase50.toml`。该脚本覆盖 access log 增长、PidFile、USR1 graceful reopen、HUP restart、SIGTERM stop 和 log rotate 重开；其中 restart 语义不再放在阶段 2。

## 阶段 7：请求体、CGI 和动态执行路径

Apache 可以用 CGI 覆盖 nginx 静态测试没有覆盖到的 exec 和管道路径。

检查项：

- 小 POST 到 CGI，CGI 读取 stdin 并回显长度。
- 大 POST 超过内存路径但未超过限制，CGI 可读完整 body。
- `LimitRequestBody` 超限返回 413。
- CGI stdout header + body 被 httpd 正确解析。
- CGI 退出码异常时 Apache 返回对照一致错误并写 error log。
- CGI 环境变量如 `REQUEST_METHOD`、`CONTENT_LENGTH`、`SCRIPT_NAME` 正确。

重点暴露：

- `fork/execve`。
- pipe、dup2、close-on-exec。
- request body 读取、临时文件。
- 环境变量和 argv 构造。

当前落地脚本：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase70.toml`。该脚本已覆盖 CGI 请求体回显、GET/POST 环境变量、4096B 大 POST、8192B 限制下的 413，以及 CGI 异常退出的 500 行为。

## 阶段 8：模块特性扩展

按模块逐项打开，不一次启用完整默认配置。

优先项：

- `mod_mime`：`TypesConfig` 和 Content-Type。
- `mod_alias`：`Alias`、`ScriptAlias`。
- `mod_autoindex`：目录列表。
- `mod_status`：`/server-status?auto`。
- `mod_deflate`：gzip 响应。
- `mod_rewrite`：简单 rewrite 和 redirect。
- name-based virtual host：`Host` header 选择不同 DocumentRoot。

当前落地脚本：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`，对应 StarryOS 入口 `qemu/phase/qemu-riscv64-phase80.toml`。该脚本已覆盖 `mod_mime`、`mod_status`、`mod_deflate`、`mod_rewrite` 和 name-based vhost；`mod_alias` / `mod_autoindex` 已在 phase4 中通过 Apache 目录规则覆盖。

延后项：

- TLS/HTTPS，除非基础 HTTP、MPM、日志和 CGI 已稳定。
- 反向代理 upstream，避免引入第二服务端干扰定位。
- WebDAV、复杂认证、LDAP 等非首轮核心模块。

## 压力测试（独立管理）

压力测试放在 `apps/starry/apache/stress/`，不与 phase 功能测试混跑。

建议项：

- 并发 2，100 请求。
- 并发 8，1000 请求。
- keep-alive 并发连接。
- prefork 多子进程下混合 200/404/range/large。
- CGI 小 POST 循环。
- graceful reload 期间持续请求。

## Smoke 首轮验收

`smoke/apache-smoke-tests.sh` 只覆盖最小闭环：

- 安装 Apache 和 curl 成功。
- `httpd -v`、`httpd -M`、`httpd -t` 成功。
- `httpd -X` 或 `httpd -DFOREGROUND` 启动成功。
- GET `/`、GET `/missing.txt`、HEAD `/small.txt` 成功。
- keep-alive 两连请求成功。
- access log 和 error log 可写。
- stop 后无残留 httpd 进程。

成功标记建议：

```text
APACHE_RUNNER_PASSED mode=smoke
```

失败标记建议：

```text
APACHE_RUNNER_FAILED
```

## 推荐执行顺序

1. 阶段 0：环境、模块、配置解析。
2. 阶段 1：`httpd -X` 前台单进程拿到第一个 HTTP 200。
3. 阶段 3：基础静态 HTTP 语义。
4. 阶段 2.1：prefork MPM。
5. 阶段 6：stop、restart、graceful、日志和 pid。
6. 阶段 4：目录规则、权限、Alias、`.htaccess`。
7. 阶段 5：大文件、sendfile、mmap、Range、条件请求。
8. 阶段 7：CGI 和请求体。
9. 阶段 2.2：worker/event MPM。
10. 阶段 8：逐项打开 Apache 模块。
11. stress：在功能阶段稳定后独立执行。

## 源码级回归测例沉淀策略

优先沉淀的 StarryOS 语义类别：

- `bug-apache-mpm-prefork-wait`：prefork 子进程、SIGCHLD、wait4。
- `bug-apache-phase20-prefork-readiness`：phase20 readiness overspecification、server-status 可读性、clean stop。
- `bug-apache-mpm-thread-futex`：worker/event 线程、futex、线程退出。
- `bug-apache-accept-mutex`：accept mutex、文件锁或 pthread mutex。
- `bug-apache-htaccess-pathwalk`：逐级目录查找、权限和 symlink。
- `bug-apache-sendfile-mmap-range`：sendfile、mmap、Range offset。
- `bug-apache-graceful-signal`：SIGUSR1/SIGHUP graceful reload。
- `bug-apache-cgi-pipe-exec`：fork/exec、pipe、dup2、CGI 环境。
- `bug-apache-log-append-reopen`：O_APPEND、rename、log reopen。

每个源码级测例应满足：

- 未修复内核上稳定失败。
- 修复后输出 `TEST PASSED`。
- 只覆盖一个明确语义，不依赖完整 Apache。
- 可通过 `--subcase` 单独运行。

## 问题归档模板

```text
标题：
Apache 阶段：
配置文件：
触发命令：
期望行为：
实际行为：
Apache error log：
Apache access log：
StarryOS log：
Linux Alpine 对照结果：
疑似 syscall/语义：
最小复现测例路径：
修复 PR/commit：
回归验证命令：
```

## 第一轮不建议投入

- TLS/HTTPS。
- 反向代理和 upstream。
- 高并发性能指标。
- 完整 Alpine 默认 Apache 配置。
- 复杂认证、LDAP、WebDAV。
- 过早多架构全量矩阵。先在单架构 QEMU 稳定定位，再扩到 riscv64、x86_64、aarch64、loongarch64。
