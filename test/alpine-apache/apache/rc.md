本 PR 新增了 `apps/starry/apache` 的 Apache Starry app 流程，包括 smoke、phase、debug/all QEMU 配置和 guest runner/prebuild 脚本。整体组织方式和 nginx app 类似，脚本也有 runner 统一封装、阶段级 PASS/FAIL marker、overlay 安装和 QEMU `success_regex`/`fail_regex`。

阻塞点是当前 head 的默认文档命令不能跑通：我在当前 head `c66b6e88ca4b6af541cc096def2ce6cd52e15d8c` 上运行 `cargo xtask starry app qemu -t apache --arch x86_64`，guest 内 `prepare packages`、`prepare apache files`、`environment probe` 和 `apache config test` 都通过，但 `start apache single process` 阶段反复输出 `sleep: invalid time interval '1'`，最终打印 `APACHE_RUNNER_PHASE_FAIL phase=smoke rc=1` 和 `APACHE_RUNNER_FAILED phase=smoke rc=1`，外层 runner 因 `fail_regex` 匹配失败退出。这个命令正是 README 中的默认 Apache app smoke 用法，因此当前 PR 不能证明新增 app 流程可用。

从日志看，失败发生在安装包之后的轮询等待路径；`APACHE_RUNNER_PKGS` 里包含 `coreutils`，当前 StarryOS guest 里安装后调用普通 `sleep 1` 会失败，导致 smoke/phase 中大量 readiness 和 cleanup loop 都不可用。建议避免让这个流程依赖当前不可用的 GNU `sleep`，例如不要安装/覆盖为 `coreutils` 的 sleep，或在共享库里显式选择并验证 `busybox sleep` 这样的可用实现，并重新跑通默认 smoke；如果 `all` 仍是 PR 声明的本地四架构验证内容，也需要至少给出当前 head 可复现的通过证据。

CI 状态方面，当前 head 的 `Check formatting / run_host`、`Run sync-lint / run_container`、`Test with std / run_host` 等已通过；Starry QEMU container jobs 是 `CANCELLED`（其中 x86_64 日志为 `context canceled`），不能作为 Apache app 通过证据。`Test axvisor self-hosted board roc-rk3568-pc-linux / run_host` 失败于 AxVisor ROC-RK3568 board U-Boot/启动路径，日志片段为 `FDT and ATAGS support not compiled in - hanging` 与 `kernel boot timed out after 300s`，该路径与本 PR 的 `apps/starry/apache/**` 变更不相交；我已把这次现象补充到既有跟踪 issue #1227。

测试/覆盖检查：新增内容是 Starry app 支持，放在 `apps/starry/apache` 这一层是合适的；但按照 app workflow 的合并要求，README/PR 正文声明的 QEMU 运行路径必须在当前 head 可运行。当前默认 smoke 已失败，因此运行时覆盖不满足。`git diff --check origin/dev...origin/pr/1311` 通过。

重复/重叠检查：base 分支没有现有 Apache app 实现；搜索 open PR 后没有发现另一个 Apache/httpd/app PR 与 #1311 重复或冲突。之前 3 条关于重复 `set -eu` 的过期 review thread 已在当前 head 修复，我已确认对应 phase 文件均只剩一处 `set -eu` 并解析了这些 thread。现有 reviewer request 中已有 @ZCShou 和 @luodeb，匹配 Starry app/rootfs/test 方向。




apps/starry/apache/runner/apache-runner-lib.sh:3-6

这里把 `coreutils` 装进 guest 后，当前 head 的默认命令 `cargo xtask starry app qemu -t apache --arch x86_64` 会在 smoke 的 `start apache single process` 阶段反复输出 `sleep: invalid time interval '1'`，随后打印 `APACHE_RUNNER_FAILED phase=smoke rc=1` 并失败。也就是说 README 里的默认 Apache app smoke 现在跑不通，后续 phase 里的 readiness/cleanup loop 同样依赖 `sleep 1`，不能只靠脚本存在或 marker 配置通过审查。建议不要让此流程覆盖到当前不可用的 GNU `sleep`，或在共享库中显式选择并验证可用的 sleep 实现（例如 busybox applet），然后重新跑通默认 smoke/声明的 all 流程。