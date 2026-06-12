# Nginx on StarryOS 压测方案

## 目标

本方案使用 nginx 作为 StarryOS 的系统压力负载。压测目标是修复和增强 StarryOS 的稳定性与性能，而不是测试 nginx 自身性能。

重点观察：

- TCP/socket 生命周期：`accept`、`read`、`write`、`close`、fd 回收。
- 事件通知与调度：并发连接下是否丢唤醒、卡死、长时间无进展。
- 进程生命周期：master/worker、信号、`waitpid`、zombie 清理。
- 文件系统与 VFS：静态文件读取、日志写入、临时文件、大文件传输。
- 资源泄漏：fd、进程、内存、临时文件、日志增长异常。
- 长时间运行稳定性：多轮负载后是否 panic、hang、吞吐明显退化。

非目标：

- 不比较 nginx QPS。
- 不调优 nginx 到最高吞吐。
- 不把压测接入默认 nginx smoke CI。

硬性要求：

- 任何 stress 测试在 StarryOS 上运行前，都必须先在 Linux 环境中运行一遍。
- Linux 运行用于确认测试脚本、请求构造、断言和统计逻辑本身正确。
- Linux 运行结果作为行为基线，用于对照 StarryOS 上的状态码、响应体、连接关闭、超时、日志、进程回收等行为偏差。
- 如果测试在 Linux 上失败，不能将其作为 StarryOS 压测问题处理；必须先修正测试脚本或测试假设。

## 方案评估

### 目的导向性

原始方案方向正确：每类流量都能对应 StarryOS 的一个或多个风险路径。

- 短连接用于压 socket 创建、accept、close、fd 复用。
- keep-alive 用于压同一 fd 上的多次读写和事件就绪。
- 大文件用于压 VFS、page cache、`sendfile`、网络发送。
- 混合流量用于压 HTTP 路径切换、文件查找、range、请求体处理。
- reload/reopen/kill worker 用于压信号、调度、进程回收。
- soak 用于发现累积泄漏和定时器类问题。

需要强化的是定位能力：每个脚本必须只主压一个维度。否则一旦失败，很难判断是 socket、VFS、调度、信号还是 nginx 配置路径触发的问题。

### 可行性

方案可行，原因如下：

- 当前 phase 测试已覆盖 nginx 启动、基础 HTTP、keep-alive、慢请求头、大文件、range、POST、日志、信号、配置特性。
- `apps/starry/nginx/prebuild.sh` 可安装额外 stress 脚本到 guest overlay。
- 独立 `qemu-*-stress-*.toml` 可用独立 success/fail regex，不污染 smoke。
- 首轮可以只用 shell + `curl`，断言简单、可控、易定位。

约束：

- shell 并发不是精确 benchmark 工具，但足够暴露 StarryOS 稳定性问题。
- Busybox/Alpine 诊断工具可能不完整，诊断项应尽量可选。
- 高并发容易造成级联失败，应按 L0/L1/L2/L3 逐级放大。
- 首轮优先 x86_64，脚本稳定后再扩展 riscv64；aarch64/loongarch64 暂不作为首轮压测目标。

### 全面性

方案覆盖了 nginx on StarryOS 的主要压力面，但必须按“先单路径、后组合、再生命周期、最后长跑”的顺序实施。

推荐顺序：

- 先验证基础 harness 和短连接。
- 再验证 keep-alive 与大文件。
- 单项稳定后再跑混合流量。
- 最后加入生命周期干扰和 soak。

这样每个失败都更容易映射到 StarryOS 的可能错误原因。

## 拆分原则

压测按“主压 StarryOS 子系统路径”拆分，而不是按 nginx 功能简单堆叠。

每个脚本遵守：

- 一个主压力维度。
- 一个主要 nginx 配置；除非配置本身就是变量。
- 同一脚本内允许 L0/L1/L2/L3 递增负载，但不能改变主压力路径。
- 混合流量只能在单路径脚本稳定后引入。
- 生命周期干扰与纯流量压测分离。
- soak 与 burst 分离。

负载等级：

| 等级 | 用途 | 示例 |
| --- | --- | --- |
| L0 | harness 验证 | concurrency 2, total 100 |
| L1 | 常规定位压测 | concurrency 8, total 1000 |
| L2 | 加强复现 | concurrency 16, total 3000 |
| L3 | 探索性极限压测 | concurrency 32, total 5000 |

L0/L1 应尽量稳定可复现。L2/L3 只用于放大已有问题，不应作为默认门槛。

## 通用 Harness

每个 stress 脚本都应包含相同生命周期：

1. 清理残留 nginx。
2. 在 `/tmp/nginx-stress-*` 下准备独立目录。
3. 生成 nginx config、www、logs、pid、out 目录。
4. 通过 `/usr/bin/nginx-alpine-mirror.sh` 安装 `nginx`、`curl` 等依赖。
5. 执行 `nginx -t`。
6. 启动 nginx。
7. 等待 health check 成功。
8. 在外层 watchdog 下运行压测。
9. 压测后再次 health check。
10. 输出轻量诊断。
11. 优先 `nginx -s quit` 退出。
12. graceful 失败后再 kill 残留进程。
13. 检查无 nginx zombie 或异常残留。
14. 输出唯一通过标记。

每个脚本还应支持 Linux 本地运行，至少保证：

- 不依赖 StarryOS 专有路径。
- 依赖安装步骤可跳过或替换为 Linux 已安装的 `nginx`、`curl`。
- Linux 与 StarryOS 使用相同 nginx config、请求序列和断言逻辑。
- Linux 基线输出与 StarryOS 输出使用相同计数格式，方便直接比较。

通过标记格式：

```text
NGINX_STRESS_<ID>_TEST_PASSED
```

失败标记格式：

```text
NGINX_STRESS_<ID>_TEST_FAILED
```

日志格式：

```text
NGINX_STRESS_<ID>_LOG: key=value ...
```

## 观测项

必需观测：

- 请求总数。
- 预期成功数。
- 失败数。
- HTTP 状态分布。
- post-stress health check 结果。
- nginx master/worker 数量。
- graceful quit 是否成功。

可选观测：

- nginx error log。
- access log 行数。
- `/proc/meminfo` 前后快照。
- nginx 进程 fd 数。
- temp 目录文件数。
- `ps` 中 nginx/zombie 状态。

可选观测不能导致测试失败，除非该脚本明确测试对应行为。

## 失败归因矩阵

| 现象 | 可能 StarryOS 问题 | 优先检查 |
| --- | --- | --- |
| 连接被拒绝 | nginx 崩溃、listen socket 状态、调度问题 | S0/S1、进程列表、error log |
| curl 超时但 nginx 存活 | 事件通知、socket wakeup、调度饥饿 | S1/S2，降低并发重跑 |
| body 截断 | VFS、page cache、`sendfile`、socket write | S3，对比 sendfile off/on |
| 简单路径状态码错误 | 请求解析或文件查找异常 | phase2、S0、S4 |
| worker 未恢复 | 信号、调度、进程创建 | S5、phase7 |
| quit 后 zombie | `waitpid`、进程回收、信号投递 | S5、phase7 |
| access log 行数异常 | 文件写入、日志 reopen、buffer flush | S4/S5、phase6 |
| 多轮后才失败 | fd/内存/定时器泄漏 | S6，加 fd/meminfo 快照 |
| 仅高并发 panic | socket/event/scheduler race | S1/S2/S4，按 L0-L3 二分 |

## 压测脚本规划

### S0 Baseline

路径：`apps/starry/nginx/stress/nginx-stress-s0-baseline.sh`

主压路径：基础 socket accept/read/write/close。

负载：

- `worker_processes 1`。
- `sendfile off`。
- 请求 `/small.txt`。
- L0：concurrency 2，total 100。

目的：验证压测 harness、qemu entry、日志与清理逻辑。

通过条件：100 个 200；压测后 health check 成功；graceful quit 成功；无 zombie。

### S1 Short Connection Churn

路径：`apps/starry/nginx/stress/nginx-stress-s1-short-conn.sh`

主压路径：socket 生命周期与 fd 回收。

负载：

- `worker_processes 1`。
- `sendfile off`。
- 每次请求新建连接。
- L1：concurrency 8，total 1000。
- L2 可选：concurrency 16，total 3000。

目的：定位 accept/close/fd 复用、事件通知、调度公平性问题。

通过条件：所有请求 200；失败数 0；压测后可继续服务；graceful quit 成功。

### S2 Keep-Alive

路径：`apps/starry/nginx/stress/nginx-stress-s2-keepalive.sh`

主压路径：同一 fd 上的多次请求与事件就绪。

负载：

- `worker_processes 1`。
- `keepalive_timeout 5`。
- L1：8 个并发客户端，每个连接 20 次请求。
- L2 可选：16 个并发客户端，每个连接 50 次请求。

目的：区分 keep-alive 状态机问题与短连接 churn 问题。

通过条件：所有响应 200；连接最终关闭；后续普通 curl 成功。

### S3 Large File Transfer

路径：`apps/starry/nginx/stress/nginx-stress-s3-large-file.sh`

主压路径：VFS、page cache、网络发送、`sendfile`。

负载：

- 子项 A：`sendfile off`。
- 子项 B：`sendfile on`。
- 请求 `/large.bin`，初始 1 MiB。
- L1：concurrency 4，total 100。
- L2 可选：concurrency 8，total 300。

目的：对比普通 read/write 与 sendfile 路径，定位截断、阻塞、缓存或发送路径问题。

通过条件：body size 正确；抽样 `cmp` 一致；无 curl 18 截断；压测后服务存活。

### S4 Mixed HTTP Paths

路径：`apps/starry/nginx/stress/nginx-stress-s4-mixed.sh`

主压路径：HTTP 路径切换和 socket/VFS 组合行为。

流量：

- `GET /small.txt` -> 200。
- `GET /missing.txt` -> 404。
- `GET /large.bin` + range -> 206。
- `GET /large.bin` -> 200。
- 超过限制的 POST -> 413。

负载：L1 concurrency 8，total 1000，按固定 modulo 选择请求类型。

目的：在单路径稳定后暴露组合路径问题。

通过条件：每类状态码符合预期；404/413 计为预期成功；无非预期 5xx、timeout、empty response。

### S5 Lifecycle Under Load

路径：`apps/starry/nginx/stress/nginx-stress-s5-lifecycle.sh`

主压路径：信号、调度、进程回收、worker 恢复。

负载：

- `master_process on`。
- `worker_processes 2`，如果目标架构稳定。
- 后台运行 S1 风格短连接压力。
- 压测中执行 `nginx -s reload`。
- 压测中执行 `nginx -s reopen`。
- L2 可选：kill 一个 worker，验证 master 拉起替代 worker。

目的：验证服务负载下的进程生命周期语义。

通过条件：reload/reopen 成功；服务不中断到不可恢复；最终 quit 后无 zombie/残留 worker。

### S6 Soak

路径：`apps/starry/nginx/stress/nginx-stress-s6-soak.sh`

主压路径：长期资源稳定性。

负载：

- 中低并发。
- 循环 small、keep-alive、large-file 请求。
- 初始持续 5 分钟。

目的：发现 burst 测试看不到的 fd、内存、timer、缓存类累积问题。

通过条件：周期性 health check 全部通过；失败率不随时间增加；结束后 graceful quit 成功；可选 fd/meminfo 无明显无界增长。

## QEMU Entry 规划

在新增或修改任何 qemu stress entry 前，必须先完成对应脚本的 Linux 基线运行。

Linux 基线应记录：

- 命令行。
- nginx 版本。
- 测试负载等级。
- 请求总数、成功数、失败数、状态码分布。
- 是否存在超时、截断、非预期状态码、残留进程或 zombie。
- 与预期行为不一致的地方。

只有 Linux 基线通过后，才能把同一脚本接入 StarryOS qemu stress entry。

首轮 x86_64：

```text
apps/starry/nginx/qemu-x86_64-stress-s0.toml
apps/starry/nginx/qemu-x86_64-stress-s1.toml
apps/starry/nginx/qemu-x86_64-stress-s2.toml
apps/starry/nginx/qemu-x86_64-stress-s3.toml
apps/starry/nginx/qemu-x86_64-stress-s4.toml
apps/starry/nginx/qemu-x86_64-stress-s5.toml
apps/starry/nginx/qemu-x86_64-stress-s6.toml
```

脚本稳定后补 riscv64：

```text
apps/starry/nginx/qemu-riscv64-stress-s0.toml
apps/starry/nginx/qemu-riscv64-stress-s1.toml
apps/starry/nginx/qemu-riscv64-stress-s3.toml
```

运行示例：

```bash
cargo xtask starry app qemu -t nginx --arch x86_64 --qemu-config apps/starry/nginx/qemu-x86_64-stress-s1.toml
```

每个 qemu entry 应使用独立判定标记。

success regex 示例：

```text
(?m)^NGINX_STRESS_S1_TEST_PASSED\s*$
```

fail regex 至少应覆盖：

- 脚本自己的失败标记，例如 `(?m)^NGINX_STRESS_S1_TEST_FAILED`。
- Starry 或内核 panic，例如 `(?i)\bpanic(?:ked)?\b`。
- 需要时再增加稳定可复现的 nginx fatal startup error，不应匹配普通 404/413 等预期 HTTP 结果。

## 实施顺序

1. S0 baseline。
2. S1 short connection churn。
3. S3 large file transfer。
4. S2 keep-alive。
5. S4 mixed HTTP paths。
6. S5 lifecycle under load。
7. S6 soak。

原因：S4/S5/S6 失败通常是组合问题，必须先让 S1/S2/S3 形成可回归基线，否则无法定位 Starry 错误原因。

## Triage 流程

压测失败时：

1. 先确认同一脚本、同一负载等级在 Linux 上是否通过。
2. 如果 Linux 失败，优先修正测试脚本或测试假设。
3. 如果 Linux 通过而 StarryOS 失败，记录 Linux 与 StarryOS 的行为偏差。
4. 降到上一负载等级重跑 StarryOS。
5. 重跑最近的 phase 测试。
6. socket 类问题对比 S1 与 S2。
7. 传输类问题对比 S3 的 `sendfile off/on`。
8. 生命周期问题对比 phase7 与 S5。
9. 查看 nginx error log、Starry panic 输出、进程列表。
10. 需要额外探针时，在 `apps/starry/nginx/debug/` 新增 issue-focused 脚本。
11. 确认根因后再更新 tracker 或 issue 文档。

## 管理规则

- stress 独立管理，不进入默认 smoke。
- 每个 stress item 必须脚本和 qemu entry 都通过后才能标记通过。
- 每个 stress item 必须先有 Linux 基线通过记录，再运行 StarryOS qemu stress。
- 不用 nginx 配置规避 StarryOS 问题，除非该配置用于隔离变量。
- 压测输出要服务于定位 StarryOS 错误原因，而不是展示 nginx 性能数字。
- `www/d2/nginx-test-tracker.md` 已将阶段 8 交给 `apps/starry/nginx/stress/` 独立管理；本方案是该独立压测工作的规划文档。
