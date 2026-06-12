## Review 结论

需要修改后再合入。

### 阻塞问题

本 PR 给 nginx app 新增了 `qemu-aarch64-phase1-2.toml` / `qemu-aarch64-phase1-3.toml` 以及 `qemu-loongarch64-phase1-2.toml` / `qemu-loongarch64-phase1-3.toml`，但没有提供默认的 `qemu-aarch64.toml` / `qemu-loongarch64.toml`。

在当前 `dev` 的 Starry app runner 中，`starry app qemu --all --arch <arch>` 会用 `qemu_app_supports_arch()` 判断：只要存在 `qemu-<arch>-*.toml` 变体，就认为该 app 支持这个架构并纳入 `--all`。真正运行时 `resolve_qemu_config()` 又会优先要求默认 `qemu-<arch>.toml`；如果只有变体配置，就直接报错，导致整批 scheduled app smoke 被 nginx 中断。

我在把 PR head 合到最新 `origin/dev` 的本地 worktree 上复现了：

```bash
cargo xtask starry app qemu -t nginx --arch aarch64
# Error: Starry app `nginx` does not provide `qemu-aarch64.toml`; pass --qemu-config ...

cargo xtask starry app qemu -t nginx --arch loongarch64
# Error: Starry app `nginx` does not provide `qemu-loongarch64.toml`; pass --qemu-config ...
```

这会影响 #1078 合入后的 `starry-apps.yml` 每架构 `cargo xtask starry app qemu --all --arch ...` 定时流程。建议补齐默认 `qemu-aarch64.toml` / `qemu-loongarch64.toml`（通常跑 smoke），或调整 phase-only 变体的放置/发现方式，确保 `--all --arch aarch64` 和 `--all --arch loongarch64` 不会因为 nginx 失败。

### 其他检查

- 已 resolve 旧 review 中已修复/已过时的线程。
- CI 当前全绿，无失败 check。
- 本地检查：`find apps/starry/nginx -name '*.sh' -print0 | xargs -0 -n1 sh -n` 通过；`git diff --check origin/dev...HEAD -- apps/starry/nginx apps/starry/README.md` 通过。
- `prebuild.sh` 安装项与各 `qemu-*.toml` 的 `shell_init_cmd` 对应关系已检查，未发现缺失。
- 未发现 `[patch.crates-io]`。
- 重叠分析：#1014 已合入，是本 PR 的前置 nginx app 基础；#1078 已合入后改变了 Starry app runner 的 `--all --arch` 行为，本 PR 需要适配；#1018 是 nginx 相关内核语义修复，和本 PR 测试补充互补，不构成重复实现。
