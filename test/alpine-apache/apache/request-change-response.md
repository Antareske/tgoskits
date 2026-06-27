# Apache PR 反馈处理说明

## 修改内容

- 在 `apps/starry/apache/runner/apache-runner-lib.sh` 中新增 `apache_runner_sleep`。
- `apache_runner_sleep` 优先使用 `busybox sleep`，仅在 busybox applet 不可用时回退到普通 `sleep`。
- 将 Apache smoke、phase、debug 脚本中的等待和清理轮询统一改为 `apache_runner_sleep 1`。
- 该修改避免 guest 安装 `coreutils` 后，测试流程依赖可能不可用的普通 `sleep` 命令。

## 本地验证结果

- 修改过的 Apache shell 脚本均通过 `sh -n` 语法检查。
- `apps/starry/apache` 下已无裸 `sleep 1` 等待调用。
- 清理 `target` 和 `tmp/axbuild` 后，x86_64 默认 smoke 通过：
  `cargo xtask starry app qemu -t apache --arch x86_64`。
- 清理后重建流程中，x86_64 all 配置通过：
  `cargo xtask starry app qemu -t apache --arch x86_64 --qemu-config apps/starry/apache/qemu/all/qemu-x86_64.toml`。
- 四个架构的 smoke 已完成手动验证并通过：x86_64、riscv64、aarch64、loongarch64。
