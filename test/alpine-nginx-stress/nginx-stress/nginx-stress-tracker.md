# Nginx 压测任务跟踪

本文追踪 nginx on StarryOS 独立压测进度。**测试通过才勾选**。

本压测任务是 `www/d2/nginx-test-tracker.md` 中功能性 normal 任务的后续压力任务，当前聚焦压测本身，不回填到 normal 阶段清单。

## 当前状态

- 已通过压测项：3/7。
- 当前 `apps/starry/nginx/stress/` 已集成 S0-S2，S3-S6 尚未集成。
- 当前已集成 `apps/starry/nginx/qemu-x86_64-stress-s0.toml`、`apps/starry/nginx/qemu-x86_64-stress-s1.toml` 与 `apps/starry/nginx/qemu-x86_64-stress-s2.toml`，S3-S6 qemu stress entry 尚未集成。
- 每个 stress item 必须先有 Linux 基线通过记录，再运行 StarryOS qemu stress。
- 每个 stress item 必须脚本和 qemu entry 都通过后才能标记通过。

## 阶段任务清单

### S0 Baseline

- [x] 基础压测 harness 与短连接 L0 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s0-baseline.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s0.toml`
  - Linux 基线：2026-06-03 通过，`apps/starry/nginx/stress/nginx-stress-s0-baseline.sh`
  - StarryOS：2026-06-03 x86_64 通过，`cargo xtask starry app qemu -t nginx --arch x86_64 --qemu-config apps/starry/nginx/qemu-x86_64-stress-s0.toml`
  - 通过标记：`NGINX_STRESS_S0_TEST_PASSED`
  - 通过条件：100 个 `/small.txt` 请求返回 200；压测后 health check 成功；graceful quit 成功；无 zombie
  - 备注：Linux 基线 nginx/1.24.0 (Ubuntu)，StarryOS guest nginx/1.28.3；L0 concurrency 2、total 100、success 100、failed 0、status_200 100，graceful quit 与残留检查通过

### S1 Short Connection Churn

- [x] 短连接 churn L1 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s1-short-conn.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s1.toml`
  - Linux 基线：2026-06-03 通过，`apps/starry/nginx/stress/nginx-stress-s1-short-conn.sh`
  - StarryOS：2026-06-05 x86_64 通过，`cargo xtask starry app qemu -t nginx --arch x86_64 --qemu-config apps/starry/nginx/qemu-x86_64-stress-s1.toml`
  - 通过标记：`NGINX_STRESS_S1_TEST_PASSED`
  - 通过条件：concurrency 8、total 1000；所有请求 200；失败数 0；压测后可继续服务；graceful quit 成功
  - 备注：Linux 基线 nginx/1.24.0 (Ubuntu)，L1 concurrency 8、total 1000、success 1000、failed 0、status_200 1000，graceful quit 与残留检查通过。StarryOS guest nginx/1.28.3 首轮 300s watchdog 下 success 571、failed 429；之后把稳态发送、收尾、重试和 pacing 完整修正，最终通过时 success 1000、failed 0。

### S2 Keep-Alive

- [x] keep-alive 并发连接 L1 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s2-keepalive.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s2.toml`
  - Linux 基线：2026-06-06 通过，`apps/starry/nginx/stress/nginx-stress-s2-keepalive.sh`
  - StarryOS：2026-06-06 x86_64 通过，`cargo xtask starry app qemu -t nginx --arch x86_64 --qemu-config apps/starry/nginx/qemu-x86_64-stress-s2.toml`
  - 通过标记：`NGINX_STRESS_S2_TEST_PASSED`
  - 通过条件：8 个并发客户端，每个连接 20 次请求；所有响应 200；连接最终关闭；后续普通 curl 成功
  - 备注：Linux 基线 nginx/1.24.0 (Ubuntu)，L1 clients 8、requests_per_client 20、success 8、failed 0。StarryOS 首版 nc keep-alive 客户端丢尾包，改为 curl 多 transfer 串行复用同一连接后通过，最终输出 8/8 全通过。

### S3 Large File Transfer

- [ ] 大文件并发传输 L1 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s3-large-file.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s3.toml`
  - Linux 基线：未记录通过
  - StarryOS：未记录通过
  - 通过标记：`NGINX_STRESS_S3_TEST_PASSED`
  - 通过条件：`sendfile off` 与 `sendfile on` 子项均通过；请求 1 MiB `/large.bin`；concurrency 4、total 100；body size 正确；抽样 `cmp` 一致；无 curl 18 截断；压测后服务存活
  - 备注：用于对比普通 read/write 与 sendfile 路径，定位 VFS、page cache、网络发送或截断问题

### S4 Mixed HTTP Paths

- [ ] 混合 HTTP 路径 L1 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s4-mixed.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s4.toml`
  - Linux 基线：未记录通过
  - StarryOS：未记录通过
  - 通过标记：`NGINX_STRESS_S4_TEST_PASSED`
  - 通过条件：concurrency 8、total 1000；`/small.txt` 为 200；`/missing.txt` 为 404；range 为 206；`/large.bin` 为 200；超过限制 POST 为 413；无非预期 5xx、timeout、empty response
  - 备注：只能在 S1/S2/S3 单路径稳定后引入，404/413 属于预期成功

### S5 Lifecycle Under Load

- [ ] 负载下生命周期干扰跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s5-lifecycle.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s5.toml`
  - Linux 基线：未记录通过
  - StarryOS：未记录通过
  - 通过标记：`NGINX_STRESS_S5_TEST_PASSED`
  - 通过条件：后台短连接压力下 `nginx -s reload` 成功；`nginx -s reopen` 成功；服务不中断到不可恢复；最终 quit 后无 zombie/残留 worker
  - 备注：主压信号、调度、进程回收和 worker 恢复；kill worker 作为 L2 可选放大项

### S6 Soak

- [ ] 长时间 soak 跑通
  - 测例：`apps/starry/nginx/stress/nginx-stress-s6-soak.sh`
  - QEMU：`apps/starry/nginx/qemu-x86_64-stress-s6.toml`
  - Linux 基线：未记录通过
  - StarryOS：未记录通过
  - 通过标记：`NGINX_STRESS_S6_TEST_PASSED`
  - 通过条件：初始持续 5 分钟；周期性 health check 全部通过；失败率不随时间增加；结束后 graceful quit 成功；可选 fd/meminfo 无明显无界增长
  - 备注：用于发现 burst 测试看不到的 fd、内存、timer、缓存类累积问题

## QEMU Entry 清单

### x86_64 首轮

- [x] `apps/starry/nginx/qemu-x86_64-stress-s0.toml`
  - success regex：`(?m)^NGINX_STRESS_S0_TEST_PASSED\s*$`
- [x] `apps/starry/nginx/qemu-x86_64-stress-s1.toml`
  - success regex：`(?m)^NGINX_STRESS_S1_TEST_PASSED\s*$`
  - 备注：2026-06-05 x86_64 通过，最终结果 success 1000、failed 0
- [x] `apps/starry/nginx/qemu-x86_64-stress-s2.toml`
  - success regex：`(?m)^NGINX_STRESS_S2_TEST_PASSED\s*$`
  - 备注：2026-06-06 x86_64 通过，最终结果 success 8、failed 0
- [ ] `apps/starry/nginx/qemu-x86_64-stress-s3.toml`
  - success regex：`(?m)^NGINX_STRESS_S3_TEST_PASSED\s*$`
- [ ] `apps/starry/nginx/qemu-x86_64-stress-s4.toml`
  - success regex：`(?m)^NGINX_STRESS_S4_TEST_PASSED\s*$`
- [ ] `apps/starry/nginx/qemu-x86_64-stress-s5.toml`
  - success regex：`(?m)^NGINX_STRESS_S5_TEST_PASSED\s*$`
- [ ] `apps/starry/nginx/qemu-x86_64-stress-s6.toml`
  - success regex：`(?m)^NGINX_STRESS_S6_TEST_PASSED\s*$`

### riscv64 后续补充

- [ ] `apps/starry/nginx/qemu-riscv64-stress-s0.toml`
  - 前置：x86_64 S0 稳定后补充
- [ ] `apps/starry/nginx/qemu-riscv64-stress-s1.toml`
  - 前置：x86_64 S1 稳定后补充
- [ ] `apps/starry/nginx/qemu-riscv64-stress-s3.toml`
  - 前置：x86_64 S3 稳定后补充

## Linux 基线记录

每个条目在 StarryOS 运行前必须补齐 Linux 基线记录。记录至少包含：命令行、nginx 版本、负载等级、请求总数、成功数、失败数、状态码分布、超时/截断/非预期状态码、残留进程或 zombie。

- [x] S0 Linux baseline
  - 记录：2026-06-03 通过，命令 `apps/starry/nginx/stress/nginx-stress-s0-baseline.sh`；nginx/1.24.0 (Ubuntu)；L0 concurrency 2、total 100、success 100、failed 0、status_200 100；无超时、截断、非预期状态码、残留进程或 zombie
- [x] S1 Linux baseline
  - 记录：2026-06-03 通过，命令 `apps/starry/nginx/stress/nginx-stress-s1-short-conn.sh`；nginx/1.24.0 (Ubuntu)；L1 concurrency 8、total 1000、success 1000、failed 0、status_200 1000；无超时、截断、非预期状态码、残留进程或 zombie
- [x] S2 Linux baseline
  - 记录：2026-06-06 通过，命令 `apps/starry/nginx/stress/nginx-stress-s2-keepalive.sh`；nginx/1.24.0 (Ubuntu)；L1 clients 8、requests_per_client 20、success 8、failed 0；无超时、截断、非预期状态码、残留进程或 zombie
- [ ] S3 Linux baseline
  - 记录：未补
- [ ] S4 Linux baseline
  - 记录：未补
- [ ] S5 Linux baseline
  - 记录：未补
- [ ] S6 Linux baseline
  - 记录：未补

## 运行与更新规则

- 不跑通不打勾；存在脚本、存在 qemu entry 或曾经启动过都不能视为通过。
- Linux 基线失败时，先修测试脚本或测试假设，不归因到 StarryOS。
- Linux 基线通过且 StarryOS 失败时，记录两端行为偏差，再按 L0/L1/L2/L3 降级或放大复现。
- S4/S5/S6 失败通常是组合问题，必须先对照 S1/S2/S3 的单路径结果。
- 压测输出用于定位 StarryOS 稳定性与性能问题，不用于比较 nginx QPS。
- stress 独立管理，不接入默认 nginx smoke CI。
