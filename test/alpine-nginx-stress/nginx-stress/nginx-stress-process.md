# Nginx 压测过程记录

本文记录 `www/nginx-stress` 独立压测的实施过程、问题定位和修复结果。只记录已实际执行的内容。

## 2026-06-03

### S0 Baseline

- 新增 `apps/starry/nginx/stress/nginx-stress-s0-baseline.sh`。
- 新增 `apps/starry/nginx/qemu-x86_64-stress-s0.toml`。
- 将 S0 脚本安装到 guest overlay。
- Linux baseline 通过。
  - nginx 版本：`nginx/1.24.0 (Ubuntu)`。
  - 结果：`100/100`，`failed=0`，`status_200=100`。
  - graceful quit 通过，残留检查通过。
- StarryOS x86_64 qemu 通过。
  - guest nginx 版本：`nginx/1.28.3`。
  - 结果：`100/100`，`failed=0`，`status_200=100`。
  - gracefull quit 通过，残留检查通过。

### S1 Short Connection Churn

- 新增 `apps/starry/nginx/stress/nginx-stress-s1-short-conn.sh`。
- 新增 `apps/starry/nginx/qemu-x86_64-stress-s1.toml`。
- 将 S1 脚本安装到 guest overlay。
- Linux baseline 通过。
  - nginx 版本：`nginx/1.24.0 (Ubuntu)`。
  - 结果：`1000/1000`，`failed=0`，`status_200=1000`。
  - graceful quit 通过，残留检查通过。
- StarryOS x86_64 首轮失败。
  - 300s watchdog：`success=571`，`failed=429`。
  - 结论：单请求完成速度明显慢于 Linux，脚本过早超时。
- StarryOS x86_64 第二轮失败。
  - watchdog 放宽至 1200s 后：`success=977`，`failed=23`。
  - 结论：不是 panic，也不是全部请求失败，而是短连接在高并发下仍有少量请求未完成，当前不能视为通过。

### 当前判断

- S0 已通过。
- S1 Linux 基线已通过。
- S1 StarryOS 仍未通过，优先继续定位请求超时与少量失败的根因，再决定是否调整 watchdog、单请求超时或并发策略。

## 2026-06-05

### S1 Final Pass

- 将 S1 short connection 的稳定性问题继续收敛到可接受范围。
- 最终 StarryOS x86_64 结果为 `success=1000`、`failed=0`，通过。

## 2026-06-06

### S2 Keep-Alive

- 新增 `apps/starry/nginx/stress/nginx-stress-s2-keepalive.sh`。
- 新增 `apps/starry/nginx/qemu-x86_64-stress-s2.toml`。
- 将 S2 脚本安装到 guest overlay。
- 首版 keep-alive 客户端使用 `nc` 流式输入，Linux 基线能跑通，但 StarryOS 上出现尾包丢失，单轮结果为 `success=5`、`failed=3`。
- 为定位问题，补充了失败样本日志，发现部分客户端会少 1 个响应，说明问题集中在 keep-alive 客户端封包/收尾而非 nginx panic。
- 随后将客户端改为单个 `curl` 进程串行发起 20 个 transfer，每个 transfer 都显式使用 `--next` 和 `-o /dev/null`，把 keep-alive 复用和结果输出分离。
- Linux baseline 通过。
  - nginx 版本：`nginx/1.24.0 (Ubuntu)`。
  - 结果：`8/8`，`failed=0`。
  - graceful quit 通过，残留检查通过。
- StarryOS x86_64 通过。
  - guest nginx 版本：`nginx/1.28.3`。
  - 结果：`8/8`，`failed=0`。
  - graceful quit 通过，残留检查通过。

### 当前判断

- S0-S2 已通过。
- 之后优先推进 S3 large file transfer。
