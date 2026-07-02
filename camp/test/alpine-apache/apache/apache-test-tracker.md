# Apache 测试任务跟踪

本文追踪 Apache httpd on StarryOS 测试进度。**只有 StarryOS 上测试通过才勾选**。

## 跟踪规则

- 每个 StarryOS 用例在执行前，必须先在 Linux Alpine 环境中运行同一脚本或同等命令，确认测试本身正确。
- Linux 结果只作为对照，不代表 StarryOS 通过；tracker 勾选只看 StarryOS 结果。
- 若 Linux 与 StarryOS 行为不同，先记录 Linux 行为、StarryOS 行为和疑似 syscall/语义，再决定是修测试还是修 StarryOS。
- 响应码、header、日志和进程退出行为以 Linux Alpine Apache 为对照，不用 nginx 行为替代 Apache 对照。
- 阻塞问题进入 `apps/starry/apache/debug/` 探针，并尽量沉淀为 `test-suit/starryos` 下的最小源码级回归测例。

## Linux 对照记录

- [x] smoke：`apache-smoke-tests.sh` 在 Linux Alpine 上通过
  - 命令：`sh apps/starry/apache/smoke/apache-smoke-tests.sh`
  - 备注：2026-06-03 在 riscv64 Alpine rootfs + qemu-user chroot 中通过；因当前宿主不允许 bind mount `/proc`，使用最小 `/proc/self/*` 占位，因此该结果只作为 Apache 配置/HTTP/log/exit 行为对照，不作为 procfs 语义对照。首次对照发现 Apache 默认 `mpm-accept` mutex 在 qemu-user 下返回 `ENOSYS`，smoke 已显式配置 `Mutex fcntl:/tmp/apache-tests/run mpm-accept`。
- [x] phase30：`apache-3-0-http-static-tests.sh` 在 Linux Alpine 上通过
  - 命令：`sh apps/starry/apache/phase/apache-3-0-http-static-tests.sh`
  - 备注：2026-06-03 在 riscv64 Alpine rootfs + qemu-user chroot 中通过；确认 Linux Apache 对照行为：GET `/dir` 返回 301，未知方法 `BAD` 返回 501。
- [x] phase40：`apache-4-0-directory-access-tests.sh` 在 Linux Alpine 上通过
  - 命令：`sh apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`
  - 备注：2026-06-03 在 riscv64 Alpine rootfs + qemu-user chroot 中通过；确认 Linux Apache 对照行为：禁用 `Indexes` 返回 403，`Require all denied` 返回 403，禁用 `FollowSymLinks` 的 symlink 返回 403，`ErrorDocument 404` 保持 404 状态并返回自定义 body。

## 阶段任务清单

### 阶段 0：环境和配置解析

- [x] `httpd -v` / `httpd -V` 可执行
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `probe_environment()`
- [x] `httpd -M` 可列出 MPM 和模块
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `probe_environment()`
- [x] `httpd -t -f smoke.conf` 配置校验通过
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_config()`
- [x] `/tmp`、runtime、log、out 目录可写
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `probe_environment()` / `prepare_tree()`
- [x] `/dev/null`、`/proc/self/fd`、`/proc/self/stat` 可访问
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `probe_environment()`
  - 备注：2026-06-03 riscv64 StarryOS smoke 通过

### 阶段 1：前台单进程闭环

- [x] `httpd -X -f smoke.conf` 前台启动成功
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `start_httpd()`
- [x] 监听 `127.0.0.1:8080`
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `start_httpd()` curl 间接验证
- [x] GET `/` 返回 200 和固定 body
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_get_index()`
- [x] GET `/missing.txt` 返回 404
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_get_missing()`
- [x] HEAD `/small.txt` 返回 200 且 `Content-Length` 正确
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_head_small()`
- [ ] 同一连接 keep-alive 两个请求成功
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_keepalive_two_requests()`
  - 备注：2026-06-03 riscv64 StarryOS 中 `nc` 响应输出为空，脚本按 nginx request-change 经验记录 skip；access log 显示 `/small.txt` 与 `/empty.txt` 请求均被 Apache 处理，但该项未严格断言通过，不勾选。
- [x] access log 和 error log 可写
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `test_logs()`
- [x] SIGTERM 后 httpd 退出且无残留
  - 测例：`apps/starry/apache/smoke/apache-smoke-tests.sh`, `stop_httpd()`
  - 备注：2026-06-03 riscv64 StarryOS smoke 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64`

### 阶段 2：MPM 和进程/线程模型

- [x] prefork MPM parent/child 启动
  - 测例：`apps/starry/apache/phase/apache-2-0-mpm-prefork-tests.sh`, `test_worker_pool_ready()`
- [x] prefork child 处理请求
  - 测例：`apps/starry/apache/phase/apache-2-0-mpm-prefork-tests.sh`, `test_request_handling()`
- [x] phase20 clean stop / httpd 退出
  - 测例：`apps/starry/apache/phase/apache-2-0-mpm-prefork-tests.sh`, `test_stop_cleanup()`
  - 备注：2026-06-18 x86_64 rerun 暴露 phase20 readiness overspecification；参见 `apps/starry/apache/debug/ISSUE-001-phase20-prefork-readiness.md` 与 `apps/starry/apache/qemu/debug/qemu-x86_64-phase20-restart.toml`。2026-06-04 riscv64 phase20 通过记录仍保留。
- [ ] worker/event MPM 可启动（若 Alpine 构建支持）
  - 测例：待新增

### 阶段 3：静态 HTTP 语义

- [x] GET `/small.txt`
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_get_small()`
- [x] GET `/empty.txt`
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_get_empty()`
- [x] GET `/dir/` 返回 `DirectoryIndex`
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_get_dir_slash()`
- [x] GET `/dir` 目录 slash redirect 与 Linux Apache 对照一致
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_get_dir_redirect()`
  - 备注：Linux/StarryOS 均为 301，Location 指向 `/dir/`
- [x] 非法方法与 Linux Apache 对照一致
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_unknown_method()`
  - 备注：Linux/StarryOS 对 `BAD /` 均返回 501
- [x] `Connection: close` 行为正确
  - 测例：`apps/starry/apache/phase/apache-3-0-http-static-tests.sh`, `test_connection_close()`
  - 备注：2026-06-03 riscv64 StarryOS phase30 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase30.toml`

### 阶段 4：Apache 目录、权限和路径规则

- [x] `Options Indexes` + `mod_autoindex`
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_autoindex()`
- [x] 禁用 `Options Indexes` 的目录访问行为
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_noindex_forbidden()`
  - 备注：Linux/StarryOS 均返回 403
- [x] `Alias`
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_alias()`
- [x] `ErrorDocument`
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_error_document()`
  - 备注：Linux/StarryOS 均保持 404 状态并返回自定义 body
- [x] `Require all granted` / `Require all denied`
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_require_denied()`
  - 备注：`Require all granted` 由多项 200 路径覆盖；`Require all denied` Linux/StarryOS 均返回 403
- [x] `AllowOverride All` + `.htaccess`
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_htaccess_directory_index()`
  - 备注：当前以 `AllowOverride Indexes` + `.htaccess` 中 `DirectoryIndex` 覆盖逐级读取行为；后续可补 `AllowOverride All` 的更多 directive 矩阵
- [x] `Options FollowSymLinks` 差异
  - 测例：`apps/starry/apache/phase/apache-4-0-directory-access-tests.sh`, `test_symlink_follow_on()` / `test_symlink_follow_off()`
  - 备注：Linux/StarryOS 均表现为开启时 200，关闭时 403；2026-06-03 riscv64 StarryOS phase40 通过，命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase40.toml`

### 阶段 5：文件发送、Range 和缓存相关语义

- [x] `EnableSendfile Off` 大文件
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_sendfile_off_large()`
- [x] `EnableSendfile On` 大文件
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_sendfile_on_large()`
- [x] `EnableMMAP Off/On`
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `prepare_tree()` 中的 off/on 配置
- [x] Range `bytes=0-15`
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_range_requests()`
- [x] Range `bytes=100-199`
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_range_requests()`
- [x] Range `bytes=-64`
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_range_requests()`
- [x] `If-Modified-Since` / `ETag` / `Last-Modified`
  - 测例：`apps/starry/apache/phase/apache-5-5-sendfile-range-tests.sh`, `test_conditional_get()`
  - 备注：2026-06-05 riscv64 StarryOS phase55 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase55.toml`

### 阶段 6：日志、runtime 文件和 graceful 生命周期

- [x] `CustomLog` 每请求新增一行
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_access_log_growth()`
- [x] `PidFile` 创建和退出处理
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_pid_file_present()` / `test_stop_works()`
- [x] `apachectl -k graceful` 或 SIGUSR1 后继续服务
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_graceful_reopen()`
- [x] `apachectl -k restart` 或 SIGHUP 后重新加载配置
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_restart_works()`
- [x] `apachectl -k stop` 或 SIGTERM 后退出
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_stop_works()`
- [x] log rotate 后 graceful/reopen
  - 测例：`apps/starry/apache/phase/apache-5-0-log-lifecycle-tests.sh`, `test_graceful_reopen()`
  - 备注：2026-06-05 riscv64 StarryOS phase50 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase50.toml`

### 阶段 7：请求体、CGI 和动态执行路径

- [x] 小 POST 到 CGI
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_cgi_env_and_body()`
- [x] 大 POST 到 CGI
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_cgi_large_body()`
- [x] `LimitRequestBody` 超限返回 413
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_limit_request_body_413()`
  - 备注：Linux/StarryOS 均返回 413
- [x] CGI stdout header + body 解析
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_cgi_env_and_body()`
- [x] CGI 异常退出写 error log
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_cgi_fail_500()`
  - 备注：Linux/StarryOS 均返回 500
- [x] CGI 环境变量正确
  - 测例：`apps/starry/apache/phase/apache-7-0-cgi-tests.sh`, `test_cgi_env_and_body()` / `test_cgi_get_env()`
  - 备注：2026-06-05 riscv64 StarryOS phase70 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase70.toml`

### 阶段 8：模块特性扩展

- [x] `mod_mime`
  - 测例：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`, `test_mime_type()`
- [x] `mod_status`
  - 测例：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`, `test_status_auto()`
- [x] `mod_deflate`
  - 测例：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`, `test_deflate()`
- [x] `mod_rewrite`
  - 测例：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`, `test_rewrite()`
- [x] name-based virtual host
  - 测例：`apps/starry/apache/phase/apache-8-0-module-feature-tests.sh`, `test_name_based_vhost()`
- [x] `mod_alias`
  - 备注：已在 phase4 目录/路径规则中通过 `Alias` 语义覆盖，无需重复测试
- [x] `mod_autoindex`
  - 备注：已在 phase4 目录/路径规则中通过 `Options Indexes` + `mod_autoindex` 覆盖，无需重复测试
  - 备注：2026-06-05 riscv64 StarryOS phase80 通过；命令 `cargo xtask starry app qemu -t apache --arch riscv64 --qemu-config apps/starry/apache/qemu/phase/qemu-riscv64-phase80.toml`

### Stress

 - 已从阶段文档剥离，后续由 `apps/starry/apache/stress/` 独立管理

## 后续测试拆分原则

- 不扩展 smoke 为全量测试。
- 每个 phase 只围绕一个 Apache 阶段或一类 StarryOS 语义。
- 压测单独成测，不和功能正确性测试混跑。
- StarryOS 勾选前必须保留 Linux Alpine 对照结果。
