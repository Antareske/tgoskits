# PR 说明：补齐 nginx 非 stress 阶段剩余探针

## PR Title

`test(starry,nginx): complete remaining non-stress phase probes`

## Summary

- 新增 phase0 环境与 `rlimit` 探针脚本：`apps/starry/nginx/phase/nginx-0-0-env-rlimit-tests.sh`。
- 新增 phase0 独立 QEMU 配置：`apps/starry/nginx/qemu-x86_64-phase0.toml`，便于单独回归。
- 增强 phase6 日志/文件系统测试：补充停止后 PID 清理校验，以及 `-p` 前缀下相对路径日志/运行目录行为校验。
- 更新 nginx prebuild 注入清单，统一把 phase/debug 相关脚本安装到 rootfs。
- 调整 x86_64/riscv64 nginx 构建特性，和当前运行链路保持一致。

## 背景与问题

现有 nginx 测试在 non-stress 阶段仍有覆盖空洞：

1. phase0 缺少可执行探针，`ulimit/getrlimit/setrlimit` 相关语义缺少稳定回归入口。
2. phase6 对 PID 退出清理、prefix 相对路径日志/运行文件等关键行为覆盖不足。
3. 部分阶段脚本未统一注入镜像，导致“脚本存在但镜像不可直接执行”的维护成本偏高。

## 改动细节

### 1) 新增 phase0 探针

- 文件：`apps/starry/nginx/phase/nginx-0-0-env-rlimit-tests.sh`
- 内容：
  - 自动准备 apk 源和依赖包（nginx/curl/busybox-extras）；
  - 构造最小 nginx 配置并执行 `nginx -t`；
  - 校验 `ulimit -n` 调整前后值与可恢复性；
  - 统一通过/失败标记便于 test harness 识别。

### 2) 新增 phase0 QEMU 配置

- 文件：`apps/starry/nginx/qemu-x86_64-phase0.toml`
- 内容：
  - 固定 shell 初始化命令为 `/usr/bin/nginx-phase00-tests.sh`；
  - 提供 `success_regex` / `fail_regex`，保证阶段性结果可自动判定。

### 3) 增强 phase6 日志/文件系统覆盖

- 文件：`apps/starry/nginx/phase/nginx-6-0-log-fs-tests.sh`
- 新增校验：
  - `test_pid_removed_after_stop`：验证 `nginx -s quit` 后 PID 文件是否及时删除；
  - `prepare_prefix_conf` + `test_prefix_relative_paths`：验证 prefix 场景下 `logs/`、`run/`、`www/` 等相对路径行为。

### 4) 统一 rootfs 注入入口

- 文件：`apps/starry/nginx/prebuild.sh`
- 改动：补齐 phase00/31/32/33/41/42/43/50/60/70/90 及部分 debug 脚本安装，避免阶段脚本遗漏。

### 5) 对齐构建特性

- 文件：
  - `apps/starry/nginx/build-x86_64-unknown-none.toml`
  - `apps/starry/nginx/build-riscv64gc-unknown-none-elf.toml`
- 改动：更新平台/驱动 feature 组合，使阶段测试与当前内核配置匹配。

## 方案逻辑

本次按“补齐入口 -> 强化关键语义 -> 统一运行链路”推进：

1. 先把 phase0 能力补齐为可重复执行探针；
2. 再增强 phase6 的高价值日志/文件系统行为校验；
3. 最后统一 prebuild 注入与构建特性，降低后续阶段测试维护成本。

## Test plan

- [x] `cargo fmt`
- [x] `cargo xtask clippy --package starry-kernel`
- [x] `cargo xtask starry app run -t nginx --arch x86_64 --qemu-config apps/starry/nginx/qemu-x86_64-phase0.toml`
