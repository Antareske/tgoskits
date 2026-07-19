当前 head `5971a961a6023b1783feb8d11cdfaf910ad55cd6` 上还不能合入，默认 x86_64 app workflow 仍然失败。

已检查项：worktree 干净；`git merge-tree --write-tree origin/dev HEAD` 可合入；`git diff --check origin/dev...HEAD` 通过；`bash -n apps/starry/apache/prebuild.sh apps/starry/apache/apache-cli-tests.sh apps/starry/apache/runner/*.sh apps/starry/apache/smoke/*.sh apps/starry/apache/phase/*.sh apps/starry/apache/debug/*.sh` 通过；`cargo xtask starry app list | rg '^apache|apache'` 能发现 `apache prebuild`；相关 open PR 搜索没有发现同等 apache/httpd app 覆盖。

但是按 PR 描述和 `apps/starry/apache/qemu-x86_64.toml` 的默认命令运行 `timeout 1800s cargo xtask starry app qemu -t apache --arch x86_64`，guest 内执行 `/usr/bin/apache-runner.sh smoke` 后失败：

- `APACHE_RUNNER_PHASE_BEGIN phase=smoke`
- `APACHE_APP_STEP_PASS: prepare packages`
- `APACHE_APP_STEP_PASS: prepare apache files`
- `APACHE_APP_STEP_PASS: environment probe`
- `APACHE_APP_STEP_PASS: apache config test`
- `APACHE_APP_STEP_FAIL: start apache single process`
- `APACHE_APP_SMOKE_FAILED failures=1 status=1`
- `APACHE_RUNNER_PHASE_FAIL phase=smoke rc=1`
- `APACHE_RUNNER_FAILED`

诊断日志里 Apache 进程存在 pid，error/stdout 只有 `(92)Protocol not available: AH00076: Failed to enable APR_TCP_DEFER_ACCEPT`，但 readiness 的 curl 在 30 秒内没有成功，因此外层 `fail_regex = ["(?m)^APACHE_RUNNER_FAILED\\b"]` 正确让 xtask 返回失败。这个 PR 新增的是 `apps/starry/apache` app workflow，review 规则要求当前 head 的实际 QEMU 命令可运行；现在默认 smoke 仍然不可复现通过，所以需要先修好 readiness/网络行为，或调整 smoke 覆盖到当前 StarryOS 已支持且能稳定证明 Apache 可用的检查，并补充 current-head 的 x86_64 运行日志。

另：当前 Actions 里 `Test starry aarch64 qemu / run_container` 失败的位置是既有 `qemu-smp1/system` 分组，日志中的各个 system 子测均显示 passed 后又匹配到 `STARRY_GROUPED_TEST_FAILED`，和本 PR 的 `apps/starry/apache` 默认命令不是同一执行面；它不是这次 request changes 的主要原因。