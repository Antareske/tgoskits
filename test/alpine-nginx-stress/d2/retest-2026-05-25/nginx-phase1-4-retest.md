# nginx phase1-4 重测与进度对齐（2026-05-25）

## 本次执行

- rootfs 准备：`cargo xtask starry rootfs --arch x86_64`
- 重测命令：`cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke`
- 重复验证：同命令再次执行 1 次，结果一致。

## 2026-05-25 复测补充（代理调整后）

- 命令：`cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke`
- 结果：PASS。
- 关键变化：`apk update` 与 `apk add nginx curl busybox-extras coreutils` 恢复成功，不再出现 TLS 错误。
- 关键观察：
  - `Unimplemented syscall: io_setup` 仍出现（single process / master worker / reload worker / sendfile 实例）。
  - 超限 POST 检查从“known issue 继续执行”变为“已返回 413”：`known issue check: too large POST now returns 413`。
  - case 最终输出 `STARRY_NGINX_SMOKE_PASSED`。

## 进度对齐更新

- 代理修复后，本地重测结果已回到队友 phase 1-4 的可执行轨道。
- 相比队友记录，本次的积极信号是：超限 POST 在当前环境中已直接返回 413（不再复现 empty reply）。
- 仍需持续关注：`io_setup` 未实现问题依旧存在，但当前 smoke 路径可 fallback，不阻塞通过。

## 2026-05-25 晚间追加重测（phase1-4 再确认）

- 命令：`cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-smoke`
- 结果：PASS（`STARRY_NGINX_SMOKE_PASSED`）。
- 对齐结论：phase1-4 对应链路再次全部通过（单进程、master/worker、reload、sendfile/range、POST、短连接）。
- 关键观察：
  - `io_setup` 未实现日志仍稳定出现（worker 启动相关时机）。
  - 超限 POST 继续表现为返回 413（`known issue check: too large POST now returns 413`）。

## 重测结论

- 当前未能进入队友记录中的 phase 1-4 业务验证步骤。
- 阻塞点前移到包准备阶段（脚本第一步 `apk install nginx curl busybox-extras coreutils`）。
- 两次重测均在同一位置失败，属于稳定复现。

## 关键失败信息

- `apk update` 拉取索引时报 TLS 错误：
  - `WARNING: updating and opening https://mirrors.cernet.edu.cn/...: TLS: unspecified error`
- 随后安装失败：
  - `ERROR: unable to select packages:`
  - `nginx (no such package)`
  - `curl (no such package)`
  - `busybox-extras (no such package)`
  - `coreutils (no such package)`
- 脚本输出：`STARRY_NGINX_STEP_FAIL: apk install nginx curl busybox-extras coreutils`

## 与队友 phase 1-4 结果对齐

- phase 1（单进程 smoke）
  - 队友：PASS（可安装 nginx/curl，能启动并完成 GET/HEAD/404/keepalive/退出）。
  - 本次：未进入该阶段（被 `apk` 阻塞）。
- phase 2（master/worker + reload/quit）
  - 队友：PASS。
  - 本次：未进入该阶段。
- phase 3（sendfile/range/POST）
  - 队友：FAIL（超限 POST 期望 413，实际 empty reply，伴随 `ioctl(FIONREAD)` 问题）。
  - 本次：未进入该阶段。
- phase 4（known issue 旁路 + 短连接）
  - 队友：PASS（记录 ISSUE-001 后继续通过）。
  - 本次：未进入该阶段。

## 进度判断

- 当前进度相对队友记录是**环境层回退**，不是内核功能点（`io_setup` / `FIONREAD`）前进或后退。
- 在恢复 `apk` 可用前，无法对 phase 1-4 的功能结果做同口径复验。

## 建议的下一步（按优先级）

1. 先做 rootfs/网络链路修复复验：在同一 `x86_64` QEMU 环境内单独验证 `apk update` 与 `apk add nginx curl`。
2. 若 TLS 仍失败，优先定位镜像源与 TLS 栈可用性（证书、时间、TLS 实现、镜像可达性），避免直接混入 nginx 功能结论。
3. 环境恢复后，按同一命令重跑 `nginx-smoke`，再对照 `www/d2/nginx-test-results.md` 的 phase 1-4 逐项打勾。
