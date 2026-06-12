# Nginx 测试任务跟踪

本文追踪当前 nginx on StarryOS 测试进度。**测试通过才勾选**

## 阶段任务清单

### 阶段 0
- [x] `nginx -v` / `nginx -V` 可执行
  - 测例：`nginx-smoke-tests.sh`, `probe_environment()`
- [x] `nginx -t -c single-worker.conf`
  - 测例：`nginx-smoke-tests.sh`, `test_config()`
- [x] `/tmp` 可写
  - 测例：`nginx-smoke-tests.sh`, `probe_environment()`
- [x] `/dev/null`、`/dev/zero` 可访问
  - 测例：`nginx-smoke-tests.sh`, `probe_environment()`
- [x] `/proc/self/fd`、`/proc/self/stat`、`/proc/meminfo` 可访问
  - 测例：`nginx-smoke-tests.sh`, `probe_environment()`
- [x] `getrlimit(RLIMIT_NOFILE)` / `setrlimit` 探测
  - 测例：`apps/starry/nginx/phase/nginx-0-0-env-rlimit-tests.sh`, `test_rlimit_probe()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase0.toml`）通过，包含 `ulimit -n` 调整后 `nginx -t` 校验
### 阶段 1.1
- [x] 单进程无 master 启动成功
  - 测例：`nginx-smoke-tests.sh`, `start_nginx()`
- [x] 单进程监听 `127.0.0.1:8080`
  - 测例：`nginx-smoke-tests.sh`, curl 间接验证
  - 备注：通过 curl 成功间接验证
- [x] pid/log 文件生成
  - 测例：`nginx-smoke-tests.sh`, log 检查与 pid 间接覆盖
  - 备注：已检查 log；pid 仅间接覆盖，后续可补显式断言
- [x] 单进程 GET `/` 返回 200
  - 测例：`nginx-smoke-tests.sh`, `test_get_index()`
- [x] 单进程退出
  - 测例：`nginx-smoke-tests.sh`, `stop_nginx()`
### 阶段 1.2
- [x] master + 1 worker 启动成功
  - 测例：`apps/starry/nginx/phase/nginx-1-2-lifecycle-tests.sh`, `test_master1_lifecycle()`
- [x] worker 处理请求
  - 测例：`apps/starry/nginx/phase/nginx-1-2-lifecycle-tests.sh`, `test_master1_lifecycle()`
- [x] `SIGQUIT` / `nginx -s quit` 有序退出
  - 测例：`apps/starry/nginx/phase/nginx-1-2-lifecycle-tests.sh`, `test_master1_lifecycle()`
- [x] `reload` 后继续服务
  - 测例：`apps/starry/nginx/phase/nginx-1-2-lifecycle-tests.sh`, `test_master1_lifecycle()`
- [x] worker 退出后 master `waitpid` 回收
  - 测例：`apps/starry/nginx/phase/nginx-1-2-lifecycle-tests.sh`, `assert_no_worker_or_zombie()`
  - 备注：已加入显式 zombie/残留 worker 检查
### 阶段 1.3
- [x] master + 2 workers 启动成功
  - 测例：`apps/starry/nginx/phase/nginx-1-3-lifecycle-tests.sh`, `test_master2()`
  - 备注：已从 known-issue 旁路切回严格断言；2026-05-28 复测：x86_64/riscv64/aarch64 通过，loongarch64 存在包安装阶段不稳定（非 1.3 断言失败）
- [x] 两个 worker 都启动
  - 测例：`apps/starry/nginx/phase/nginx-1-3-lifecycle-tests.sh`, `test_master2()`
  - 备注：进程列表可观察到两个 worker；脚本内字符串匹配统计口径待修正
- [x] 多 worker accept/共享 listen fd 正常
  - 测例：`apps/starry/nginx/phase/nginx-1-3-lifecycle-tests.sh`, `test_master2()`
  - 备注：x86_64 qemu 上短轮询请求与共享监听已通过
- [x] 两 worker 均可退出并被回收
  - 测例：`apps/starry/nginx/phase/nginx-1-3-lifecycle-tests.sh`, `test_master2()`
  - 备注：严格 `nginx -s quit` 断言已启用；2026-05-28 在 x86_64/riscv64/aarch64 回归通过
### 阶段 2
- [x] GET `/`
  - 测例：`nginx-smoke-tests.sh`, GET `/` 实现
- [x] GET `/small.txt`
  - 测例：`apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh`, `test_get_small()`
  - 备注：已在 `apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh` 跑通
- [x] GET `/empty.txt`
  - 测例：`apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh`, `test_get_empty()`
  - 备注：已在 `apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh` 跑通
- [x] GET `/missing.txt` -> 404
  - 测例：`nginx-smoke-tests.sh`, `test_get_missing()`
- [x] HEAD `/small.txt`
  - 测例：`nginx-smoke-tests.sh`, `test_head_small()`
- [x] GET `/dir/` -> `/dir/index.html`
  - 测例：`apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh`, `test_get_dir_slash()`
  - 备注：已在 `apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh` 跑通
- [x] GET `/dir` -> 301/302 或符合 nginx 行为
  - 测例：`apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh`, `test_get_dir_redirect()`
- [ ] 非法方法 `BAD / HTTP/1.1` -> 400/405
  - 测例：`apps/starry/nginx/phase/nginx-2-0-http-basic-tests.sh`（当前旁路）、`apps/starry/nginx/debug/nginx-2-0-bad-method-debug.sh`、`apps/starry/nginx/debug/nginx-2-0-bad-method-matrix.sh`
  - 备注：该节点暂时搁置为 ISSUE-004，不阻塞 phase2 主流程。已确认 `curl -X BAD` 与 `nc.openbsd` 路径可稳定返回 `405`，`busybox nc` 在 BAD/GET 下可出现 `<empty>`。当前将 phase2 里的 BAD 节点改为旁路记录，后续以 debug 目录总结文档与脚本持续跟进，待根因明确后再恢复为严格断言。
### 阶段 3.1
- [x] 短连接 100 次循环成功
  - 测例：`apps/starry/nginx/phase/nginx-3-1-short-connection-tests.sh`, `test_short_connection_100()`
  - 备注：已从 smoke 的 20 次补齐为 phase 独立 100 次循环；2026-05-29 已在 x86_64/riscv64 qemu 专项入口复测通过（`qemu-x86_64-phase31.toml`、`qemu-riscv64-phase31.toml`）
### 阶段 3.2
- [x] 同一连接内两个请求 keep-alive
  - 测例：`apps/starry/nginx/phase/nginx-3-2-keepalive-tests.sh`, `test_keepalive_two_requests()`；`nginx-smoke-tests.sh`, `test_keepalive_two_requests()`
  - 备注：2026-05-29 已在 x86_64/riscv64 qemu 专项入口复测通过（`qemu-x86_64-phase32.toml`、`qemu-riscv64-phase32.toml`）
- [x] `Connection: keep-alive` / `close` 行为细化验证
  - 测例：`apps/starry/nginx/phase/nginx-3-2-keepalive-tests.sh`, `test_connection_close_behavior()`
- [x] idle timeout 后连接关闭
  - 测例：`apps/starry/nginx/phase/nginx-3-2-keepalive-tests.sh`, `test_idle_timeout_close()`
### 阶段 3.3
- [x] 慢请求头，未超时可继续解析
  - 测例：`apps/starry/nginx/phase/nginx-3-3-slow-header-tests.sh`, `test_slow_header_within_timeout()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase33.toml`）通过；分段发送请求头（段间 sleep）未超时返回 200
- [x] 慢请求头，超时后连接关闭
  - 测例：`apps/starry/nginx/phase/nginx-3-3-slow-header-tests.sh`, `test_slow_header_timeout_close()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase33.toml`）通过；`client_header_timeout 4` + 人为超时未返回 200
- [x] 慢请求不阻塞其他连接
  - 测例：`apps/starry/nginx/phase/nginx-3-3-slow-header-tests.sh`, `test_slow_header_not_block_other_conn()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase33.toml`）通过；慢连接存在时并发 curl 仍可返回
### 阶段 4.1
- [x] `sendfile off` 大文件请求
  - 测例：`apps/starry/nginx/phase/nginx-4-1-sendfile-off-tests.sh`, `test_sendfile_off_large_once()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase41.toml`）通过；`sendfile off` 下校验大小与内容一致性
- [x] `sendfile off` 大文件多次稳定性
  - 测例：`apps/starry/nginx/phase/nginx-4-1-sendfile-off-tests.sh`, `test_sendfile_off_large_stability()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase41.toml`）通过；循环 5 次下载并逐次 `cmp`
### 阶段 4.2
- [x] `sendfile on` 大文件请求
  - 测例：`nginx-smoke-tests.sh`, `test_large_sendfile()`
  - 备注：phase 独立本体已补充：`apps/starry/nginx/phase/nginx-4-2-sendfile-on-tests.sh`, `test_sendfile_on_large_once()`（与 smoke 覆盖节点重叠）。2026-05-29 x86_64（`qemu-x86_64-phase42.toml`）首轮失败（`curl: (18)` 截断），经 `apps/starry/nginx/debug/nginx-4-2-sendfile-on-debug.sh` 定向探针（5/5 为 1048576）后重跑通过。
- [x] `sendfile on` 大文件多次稳定性
  - 测例：`apps/starry/nginx/phase/nginx-4-2-sendfile-on-tests.sh`, `test_sendfile_on_large_stability()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase42.toml`）重跑通过；脚本内循环 5 次下载并逐次 `cmp` 均通过
### 阶段 4.3
- [x] Range `bytes=0-15` -> 206
  - 测例：`nginx-smoke-tests.sh`, `test_range()`
  - 备注：phase 独立本体已补充：`apps/starry/nginx/phase/nginx-4-3-range-tests.sh`, `test_range_0_15()`（与 smoke 覆盖节点重叠）；2026-05-29 x86_64（`qemu-x86_64-phase43.toml`）通过
- [x] Range `bytes=100-199`
  - 测例：`apps/starry/nginx/phase/nginx-4-3-range-tests.sh`, `test_range_100_199()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase43.toml`）通过；检查 `206`、`Content-Range: bytes 100-199/1048576` 与 body 长度 100
- [x] Range `bytes=-64`
  - 测例：`apps/starry/nginx/phase/nginx-4-3-range-tests.sh`, `test_range_suffix_64()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase43.toml`）通过；检查 `206`、`Content-Range: bytes 1048512-1048575/1048576` 与 body 长度 64
### 阶段 5
- [x] 小 POST，不崩溃
  - 测例：`nginx-smoke-tests.sh`, `test_post_small()`；`apps/starry/nginx/phase/nginx-5-0-request-body-tests.sh`, `test_post_small()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase5.toml`）通过
- [x] 大 POST，观察 client body temp file
  - 测例：`apps/starry/nginx/phase/nginx-5-0-request-body-tests.sh`, `test_post_over_buffer_path()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase5.toml`）通过；当前以“超过 buffer 的 body 语义路径可达且请求可完成”作为阶段性覆盖，temp file 细粒度观测后续补强
- [x] 超过 buffer 的 body 路径
  - 测例：`apps/starry/nginx/phase/nginx-5-0-request-body-tests.sh`, `test_post_over_buffer_path()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase5.toml`）通过
- [x] 超过 `client_max_body_size` -> 413
  - 测例：`nginx-smoke-tests.sh`, 超大 POST 413 探针；`apps/starry/nginx/phase/nginx-5-0-request-body-tests.sh`, `test_post_too_large_413()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase5.toml`）通过；phase5 已采用严格 413 断言
### 阶段 6
- [x] access log 每请求写一行
  - 测例：`nginx-smoke-tests.sh`, `test_logs()`；`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`, `test_access_log_line_growth()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase6.toml`）已补“行数增长”断言并通过
- [x] error log 可写
  - 测例：`nginx-smoke-tests.sh`, `test_logs()`；`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`, `test_error_log_writable()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase6.toml`）通过
- [x] `USR1`/`nginx -s reopen` 后重新打开日志
  - 测例：`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`, `test_reopen_logs()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase6.toml`）通过
- [x] pid 文件创建与删除
  - 测例：`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`, `test_pid_file_present()`、`test_pid_removed_after_stop()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase6.toml`）通过，已覆盖创建/存在与 `nginx -s quit` 后删除
- [x] 相对路径基于 `-p` prefix 解析
  - 测例：`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`, `test_prefix_relative_paths()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase6.toml`）通过，`logs/*` 与 `run/nginx.pid` 相对路径按 `-p` 生效
### 阶段 7
- [x] `nginx -s stop` 快速退出
  - 测例：`apps/starry/nginx/phase/nginx-7-0-signal-lifecycle-tests.sh`, `test_stop_fast_exit()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase7.toml`）通过
- [x] `nginx -s quit` 优雅退出
  - 测例：`nginx-smoke-tests.sh`, `stop_nginx_master()`
  - 备注：master 单 worker 已覆盖
- [x] `nginx -s reload` 重新加载配置
  - 测例：`nginx-smoke-tests.sh`, `test_master_reload()`；`apps/starry/nginx/phase/nginx-7-0-signal-lifecycle-tests.sh`, `test_reload_works()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase7.toml`）通过
- [x] `nginx -s reopen` 重新打开日志
  - 测例：`apps/starry/nginx/phase/nginx-7-0-signal-lifecycle-tests.sh`, `test_reopen_works()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase7.toml`）通过
- [x] worker 人为 kill，master 感知并处理
  - 测例：`apps/starry/nginx/phase/nginx-7-0-signal-lifecycle-tests.sh`, `test_worker_kill_recover()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase7.toml`）通过；kill 旧 worker 后有新 worker 拉起且服务持续可用
### 阶段 8
- [ ] 已从阶段文档剥离，统一由 `apps/starry/nginx/stress/` 独立管理
  - 测例：`apps/starry/nginx/stress/README.md`
### 阶段 9
- [x] `gzip off/on`
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_gzip_off_on()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过
- [x] `autoindex on`
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_autoindex()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过
- [x] `try_files`
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_try_files()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过
- [x] `error_page`
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_error_page()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过
- [x] `alias`
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_alias()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过
- [x] IPv6 listen
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_ipv6_listen()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过；`listen [::1]:8081` + `curl -6` 验证
- [x] Unix domain socket listen
  - 测例：`apps/starry/nginx/phase/nginx-9-0-config-feature-tests.sh`, `test_unix_socket_listen()`
  - 备注：2026-05-29 x86_64（`qemu-x86_64-phase9.toml`）通过；`listen unix:/tmp/nginx-phase90/nginx.sock` + `curl --unix-socket` 验证
### 阶段 10
- [ ] 阻塞问题按模板归档
  - 测例：-
  - 备注：文档模板已有，尚未整理到 app 目录下

## 后续测试拆分方案

原则：

- 不再扩展 `smoke`。
- 每个新测试只围绕一个阶段或一类内核/协议语义。
- 压测单独成测，不和功能正确性测试混跑。
