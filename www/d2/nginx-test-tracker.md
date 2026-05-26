# Nginx 测试任务跟踪

本文追踪当前 nginx on StarryOS 测试进度。

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
- [ ] `getrlimit(RLIMIT_NOFILE)` / `setrlimit` 探测
  - 测例：-
  - 备注：尚未做独立检查
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
  - 测例：`nginx-smoke-tests.sh`, `start_nginx_master()`
- [x] worker 处理请求
  - 测例：`nginx-smoke-tests.sh`, `test_master_get_index()`
- [x] `SIGQUIT` / `nginx -s quit` 有序退出
  - 测例：`nginx-smoke-tests.sh`, `stop_nginx_master()`
- [x] `reload` 后继续服务
  - 测例：`nginx-smoke-tests.sh`, `test_master_reload()`
- [ ] worker 退出后 master `waitpid` 回收
  - 测例：-
  - 备注：未做显式 zombie/回收断言
### 阶段 1.3
- [ ] master + 2 workers 启动成功
  - 测例：-
- [ ] 两个 worker 都启动
  - 测例：-
- [ ] 多 worker accept/共享 listen fd 正常
  - 测例：-
- [ ] 两 worker 均可退出并被回收
  - 测例：-
### 阶段 2
- [x] GET `/`
  - 测例：`nginx-smoke-tests.sh`, GET `/` 实现
- [ ] GET `/small.txt`
  - 测例：-
  - 备注：仅作为其他步骤附带访问，未单列断言
- [ ] GET `/empty.txt`
  - 测例：-
  - 备注：仅在 keep-alive 原始请求中附带使用，未单列断言
- [x] GET `/missing.txt` -> 404
  - 测例：`nginx-smoke-tests.sh`, `test_get_missing()`
- [x] HEAD `/small.txt`
  - 测例：`nginx-smoke-tests.sh`, `test_head_small()`
- [ ] GET `/dir/` -> `/dir/index.html`
  - 测例：-
  - 备注：虽准备了 `dir/index.html`，但无断言
- [ ] GET `/dir` -> 301/302 或符合 nginx 行为
  - 测例：-
- [ ] 非法方法 `BAD / HTTP/1.1` -> 400/405
  - 测例：-
### 阶段 3.1
- [ ] 短连接 100 次循环成功
  - 测例：-
  - 备注：smoke 只有 20 次，未达到计划标准
### 阶段 3.2
- [x] 同一连接内两个请求 keep-alive
  - 测例：`nginx-smoke-tests.sh`, `test_keepalive_two_requests()`
- [ ] `Connection: keep-alive` / `close` 行为细化验证
  - 测例：-
  - 备注：仅做了最小 happy path
- [ ] idle timeout 后连接关闭
  - 测例：-
### 阶段 3.3
- [ ] 慢请求头，未超时可继续解析
  - 测例：-
- [ ] 慢请求头，超时后连接关闭
  - 测例：-
- [ ] 慢请求不阻塞其他连接
  - 测例：-
### 阶段 4.1
- [ ] `sendfile off` 大文件请求
  - 测例：-
  - 备注：smoke 仅在 `sendfile on` 配置下测大文件
- [ ] `sendfile off` 大文件多次稳定性
  - 测例：-
### 阶段 4.2
- [x] `sendfile on` 大文件请求
  - 测例：`nginx-smoke-tests.sh`, `test_large_sendfile()`
- [ ] `sendfile on` 大文件多次稳定性
  - 测例：-
### 阶段 4.3
- [x] Range `bytes=0-15` -> 206
  - 测例：`nginx-smoke-tests.sh`, `test_range()`
- [ ] Range `bytes=100-199`
  - 测例：-
- [ ] Range `bytes=-64`
  - 测例：-
### 阶段 5
- [x] 小 POST，不崩溃
  - 测例：`nginx-smoke-tests.sh`, `test_post_small()`
- [ ] 大 POST，观察 client body temp file
  - 测例：-
- [ ] 超过 buffer 的 body 路径
  - 测例：-
- [x] 超过 `client_max_body_size` -> 413
  - 测例：`nginx-smoke-tests.sh`, 超大 POST 413 探针
  - 备注：仅为 known issue probe；当前不作为严格通过标准
### 阶段 6
- [x] access log 每请求写一行
  - 测例：`nginx-smoke-tests.sh`, `test_logs()`
  - 备注：已检查存在；仍建议补“行数增长”断言
- [x] error log 可写
  - 测例：`nginx-smoke-tests.sh`, `test_logs()`
- [ ] `USR1`/`nginx -s reopen` 后重新打开日志
  - 测例：-
- [ ] pid 文件创建与删除
  - 测例：-
- [ ] 相对路径基于 `-p` prefix 解析
  - 测例：-
  - 备注：现配置基本用绝对路径，未专测
### 阶段 7
- [ ] `nginx -s stop` 快速退出
  - 测例：-
- [x] `nginx -s quit` 优雅退出
  - 测例：`nginx-smoke-tests.sh`, `stop_nginx_master()`
  - 备注：master 单 worker 已覆盖
- [x] `nginx -s reload` 重新加载配置
  - 测例：`nginx-smoke-tests.sh`, `test_master_reload()`
- [ ] `nginx -s reopen` 重新打开日志
  - 测例：-
- [ ] worker 人为 kill，master 感知并处理
  - 测例：-
### 阶段 8
- [ ] 并发 2，100 请求
  - 测例：-
- [ ] 并发 8，1000 请求
  - 测例：-
- [ ] 并发 32，5000 请求
  - 测例：-
- [ ] keep-alive 并发连接
  - 测例：-
- [ ] 大文件并发下载
  - 测例：-
- [ ] 混合 200/404/range/large
  - 测例：-
### 阶段 9
- [ ] `gzip off/on`
  - 测例：-
- [ ] `autoindex on`
  - 测例：-
- [ ] `try_files`
  - 测例：-
- [ ] `error_page`
  - 测例：-
- [ ] `alias`
  - 测例：-
- [ ] IPv6 listen
  - 测例：-
- [ ] Unix domain socket listen
  - 测例：-
### 阶段 10
- [ ] 阻塞问题按模板归档
  - 测例：-
  - 备注：文档模板已有，尚未整理到 app 目录下

## 当前测试说明整理

### 已实现

- [x] nginx 第一轮冒烟闭环
  - 测例：`nginx-smoke-tests.sh`, 多函数组合实现
  - 阶段与内容：阶段 0 的环境探测与 `nginx -t`；阶段 1.1 的单进程启动/GET/退出；阶段 1.2 的 master+1 worker 启动/GET/reload/quit；阶段 2 的 `/`、404、HEAD；阶段 3 的 keep-alive 两连、20 次短连接；阶段 4 的 `sendfile on` 大文件与一个 range；阶段 5 的小 POST、超大 POST 探针；阶段 6 的 access/error log 可写；阶段 7 的 quit、reload
  - 备注：当前唯一计入覆盖的测例；仍偏“大而全”，后续不建议继续膨胀

## 后续测试拆分方案

原则：

- 不再扩展 `smoke`。
- 每个新测试只围绕一个阶段或一类内核/协议语义。
- 压测单独成测，不和功能正确性测试混跑。

### 建议新增测试清单

- [ ] HTTP 基本语义补齐
  - 测例：`nginx-http-basic-tests.sh`
  - 涵盖阶段与内容：阶段 2 的 `GET /small.txt`、`GET /empty.txt`、`GET /dir/`、`GET /dir`、非法方法 `BAD / HTTP/1.1`
  - 备注：把基础 HTTP 语义补全，便于后续问题先定位在解析/路由层
- [ ] 多 worker 启动与生命周期
  - 测例：`nginx-multiworker-lifecycle-tests.sh`
  - 涵盖阶段与内容：阶段 1.3 + 阶段 7 的 master + 2 workers 启动、worker 数量、共享 listen fd、`stop`、`reload`、worker 回收、zombie 检查
  - 备注：用于覆盖多进程生命周期语义
- [ ] keep-alive 与连接关闭
  - 测例：`nginx-keepalive-tests.sh`
  - 涵盖阶段与内容：阶段 3.2 的同连接多请求、`Connection: keep-alive/close`、idle timeout、连接关闭行为
  - 备注：重点暴露 keep-alive、FIN/RST、EAGAIN、epoll 连接状态
- [ ] 慢请求头与超时
  - 测例：`nginx-slow-header-tests.sh`
  - 涵盖阶段与内容：阶段 3.3 的分段发送请求头、未超时继续解析、超时关闭、不阻塞其他连接
  - 备注：重点暴露 timer、epoll timeout、非阻塞 socket
- [ ] sendfile off 大文件路径
  - 测例：`nginx-sendfile-off-tests.sh`
  - 涵盖阶段与内容：阶段 4.1 的 `sendfile off` 请求 `large.bin`、长度/内容校验、多次稳定性
  - 备注：与 sendfile on 分开，便于定位读写路径问题
- [ ] sendfile on 与 range 扩展
  - 测例：`nginx-sendfile-on-range-tests.sh`
  - 涵盖阶段与内容：阶段 4.2 + 4.3 的 `sendfile on` 大文件稳定性、`0-15`、`100-199`、`-64` 三种 range
  - 备注：重点暴露 sendfile offset、range 处理、短写问题
- [ ] 请求体与临时文件
  - 测例：`nginx-request-body-tests.sh`
  - 涵盖阶段与内容：阶段 5 的小 POST、大 POST、client body temp file、`client_max_body_size` 413
  - 备注：重点定位 temp file、unlink/rename、磁盘错误路径
- [ ] 日志与 prefix 路径语义
  - 测例：`nginx-log-prefix-tests.sh`
  - 涵盖阶段与内容：阶段 6 的 access log 行数增长、error log 内容、pid 文件、相对路径基于 `-p`、`reopen`
  - 备注：把日志语义和路径语义单独抽出，避免和连接测试混杂
- [ ] 配置特性基础扩展
  - 测例：`nginx-config-feature-tests.sh`
  - 涵盖阶段与内容：阶段 9 的 `autoindex`、`try_files`、`error_page`、`alias`
  - 备注：先挑不引入额外复杂依赖的配置特性
- [ ] gzip 可选特性测试
  - 测例：`nginx-gzip-tests.sh`
  - 涵盖阶段与内容：阶段 9 的 `gzip off/on`
  - 备注：若 gzip 支持路径暴露 libc/内存问题，建议单独成测
- [ ] Unix socket / IPv6 可选能力测试
  - 测例：`nginx-advanced-listen-tests.sh`
  - 涵盖阶段与内容：阶段 9 的 IPv6 listen、Unix domain socket listen
  - 备注：仅在 StarryOS 对应能力成熟后启用
- [ ] 小并发正确性测试
  - 测例：`nginx-concurrency-smoke-tests.sh`
  - 涵盖阶段与内容：阶段 8 的并发 2，100 请求；混合少量 200/404/range
  - 备注：这是“非压测”的并发正确性测试，仍以功能为主
- [ ] nginx 压力测试
  - 测例：`nginx-stress-tests.sh`
  - 涵盖阶段与内容：阶段 8 的并发 8/32、1000/5000 请求、keep-alive 并发、大文件并发、混合流量
  - 备注：明确归为压测，单独维护，失败不应与功能冒烟混淆

## 推荐阶段化推进顺序

1. `nginx-smoke-tests.sh`
内容：维持第一轮功能闭环基线，不再继续加需求。

2. `nginx-http-basic-tests.sh`
内容：补齐阶段 2 缺口，先把最基础 HTTP 语义完整化。

3. `nginx-multiworker-lifecycle-tests.sh`
内容：重建阶段 1.3 与生命周期测试。

4. `nginx-keepalive-tests.sh`
内容：聚焦阶段 3.2。

5. `nginx-slow-header-tests.sh`
内容：聚焦阶段 3.3。

6. `nginx-sendfile-off-tests.sh`
内容：先把 `read + writev/write` 路径单独测透。

7. `nginx-sendfile-on-range-tests.sh`
内容：再覆盖 sendfile 与 range 组合路径。

8. `nginx-request-body-tests.sh`
内容：聚焦阶段 5 的临时文件和 request body。

9. `nginx-log-prefix-tests.sh`
内容：日志、pid、prefix、reopen 独立验证。

10. `nginx-config-feature-tests.sh` / `nginx-gzip-tests.sh` / `nginx-advanced-listen-tests.sh`
内容：阶段 9 配置特性扩展，按依赖复杂度逐项打开。

11. `nginx-concurrency-smoke-tests.sh`
内容：小并发正确性。

12. `nginx-stress-tests.sh`
内容：压测，单独运行与维护。

## 当前完成度摘要

- 阶段 0：核心已覆盖，`rlimit` 未覆盖。
- 阶段 1：1.1、1.2 基本覆盖；1.3 目前按未测试处理。
- 阶段 2：只完成基础子集，目录与非法方法仍缺。
- 阶段 3：仅有 keep-alive 最小 happy path 和 20 次短连接；慢请求与更细连接语义未测。
- 阶段 4：已覆盖 `sendfile on` 大文件和一个 range；`sendfile off` 与 range 扩展未测。
- 阶段 5：小 POST 已测；temp file 路径基本未测；413 目前只是探针。
- 阶段 6：只确认了日志可写；`reopen`、pid、prefix 尚未正式覆盖。
- 阶段 7：只计入 `quit` 与 `reload`；`stop`、`reopen`、worker kill 未测。
- 阶段 8：全部按未测处理，且压测必须单独建测。
- 阶段 9：全部未测。
