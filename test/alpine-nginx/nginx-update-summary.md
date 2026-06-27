# Starry nginx 测试更新简要说明

本文简要说明 `apps/starry/nginx` 本轮测试入口与调试资产更新。

## 入口对齐

- 上层 CI / 默认发现入口保留在 app 根目录：`apps/starry/nginx/qemu-<arch>.toml`。
- 四个 CI smoke 命令分别为：

```bash
cargo xtask starry app qemu -t nginx --arch x86_64
cargo xtask starry app qemu -t nginx --arch riscv64
cargo xtask starry app qemu -t nginx --arch aarch64
cargo xtask starry app qemu -t nginx --arch loongarch64
```

- 这些根目录 QEMU 配置都会在 guest 内执行 `/usr/bin/nginx-runner.sh smoke`。
- `qemu/all/`、`qemu/phase/`、`qemu/debug/` 只作为手工复测入口，需要显式传 `--qemu-config`，不会被上层 CI 默认发现。

## 统一 runner

- 新增统一 guest 入口 `runner/nginx-runner.sh` 和共享库 `runner/nginx-runner-lib.sh`。
- runner 支持 `smoke`、`all`、`phase <id>`、`stress`、`debug <name>`。
- QEMU success/fail regex 统一匹配 runner 终态 marker：`NGINX_RUNNER_PASSED` / `NGINX_RUNNER_FAILED`。
- `all` 模式按 smoke + phase 顺序运行，并在每个阶段后做 cleanup、端口释放等待和临时目录隔离。

## QEMU 配置整理

- 根目录 `qemu-<arch>.toml`：CI/default smoke 入口。
- `qemu/all/qemu-<arch>.toml`：手工全量入口，运行 smoke + 全部 phase。
- `qemu/phase/qemu-<arch>-phaseXX.toml`：手工单阶段入口。
- `qemu/debug/qemu-*.toml`：问题定位入口。
- 旧的根目录散乱 phase/debug QEMU 配置已迁移到对应子目录。

## phase 与 debug 更新

- phase 脚本移除自带 watchdog，由 runner 统一负责超时控制。
- phase 脚本增加 cleanup trap，避免失败或被终止后残留 nginx 进程。
- x86_64 覆盖全部 phase 手工入口；其他架构只保留已验证过的部分 phase 入口。
- debug 目录新增 short-connection 与 x86 timing 相关调试脚本和 QEMU 配置。

## x86 phase31 记录

- 新增 `apps/starry/nginx/debug/ISSUE-005-x86-short-connection-timeout.md` 记录 x86_64 `phase31` 短连接超时调查。
- 结论指向 x86_64 上短生命周期外部命令开销偏高，而不是 nginx HTTP 响应内容错误。
- `phase31` 保持 100 次独立短连接断言，仅将每次 curl 的 wall-clock timeout 放宽到 15s。

## 文档状态

- `apps/starry/nginx/README.md`、`apps/starry/README.md` 和 `www/nginx-ci-refactor-proposal.md` 已与当前测试行为对齐。
- nginx 文档中没有保留 `cargo xtask starry app run -t nginx` 或 `nginx-runner.sh run` 入口；当前 nginx app 使用 `cargo xtask starry app qemu ...`。
