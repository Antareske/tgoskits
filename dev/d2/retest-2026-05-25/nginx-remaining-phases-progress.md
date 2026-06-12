# nginx 剩余阶段推进记录（2026-05-25）

## 目标

- 在 phase1-4 已反复通过后，继续推进队友计划中的剩余阶段（重点先做 `worker_processes 2`、生命周期、日志、小并发）。
- 遇到重要阻塞后暂停并记录。

## 执行方式

- 新增测试用例：`test-suit/starryos/normal/qemu-smp1/nginx-followup/`
  - `qemu-x86_64.toml`
  - `sh/nginx-followup-tests.sh`
- 运行命令：

```sh
cargo xtask starry test qemu --arch x86_64 -g normal -c nginx-followup
```

## 已覆盖项（计划映射）

- 阶段 1.3：`master_process on; worker_processes 2;`（启动闭环）
- 阶段 6：日志 reopen / access log 增长（已编排，尚未进入执行）
- 阶段 7：`reload`/`reopen`/`stop`（已编排，尚未进入执行）
- 阶段 8：并发 `2 x 100`（已编排，尚未进入执行）

## 关键结果

- 在 `start nginx master two workers` 阶段稳定失败，未进入后续步骤。
- 失败前可见两次 worker 相关日志：

```text
Unimplemented syscall: io_setup (tid=34)
Unimplemented syscall: io_setup (tid=35)
```

- 用例输出：`STARRY_NGINX_FOLLOWUP_STEP_FAIL: start nginx master two workers`

## 重要问题判断（本轮停止点）

- 当前重要阻塞点：**`worker_processes 2` 场景启动后服务未就绪（8083 无法在限定时间内返回 200）**。
- 该问题与 phase1-4 的单 worker 成功形成明显分水岭，符合“重要问题可暂停”条件。
- 因已达到阶段性阻塞，本轮按计划在此停止，待后续专项定位。

## 建议下一步

1. 在该 followup 用例里补充更强诊断：启动失败时输出 `ps`、`/tmp/nginx-followup/logs/error.log`、`nginx-master.log`、监听状态（`ss/netstat` 可用时）。
2. 增加对照：把 `worker_processes 2` 改回 `1`，同脚本同端口验证能否立即通过，以缩小变量到多 worker 差异。
3. 若确认是多 worker 特有问题，优先下钻 `accept/epoll/channel/wait4` 语义，并沉淀到 `bugfix` 最小复现。
