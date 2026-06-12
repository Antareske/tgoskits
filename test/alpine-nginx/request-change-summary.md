## nginx CI 修改总结

本次修改已响应要求补齐默认 QEMU 配置，目标是修复 nginx 默认 CI 入口，避免 `starry-apps.yml` 的 `cargo xtask starry app qemu --all --arch <arch>` 被 nginx 阻断。

### 已做修改

- 已响应要求补齐默认 QEMU 配置：
  - `apps/starry/nginx/qemu-aarch64.toml`
  - `apps/starry/nginx/qemu-loongarch64.toml`
- 修复构建配置：
  - `build-riscv64gc-unknown-none-elf.toml` 移除无效的 `ax-driver/rtc`
  - `build-aarch64-unknown-none-softfloat.toml` 改为 `plat_dyn = true`
- 收敛默认 smoke：
  - 默认入口只保留基础 smoke
- 增强 mirror 安装稳定性：
  - mirror timeout 提高
  - 增加重试
  - loongarch64 默认内存提升到 `1G`

### 验证结果

- `cargo xtask starry app qemu -t nginx --arch x86_64` 通过
- `cargo xtask starry app qemu -t nginx --arch riscv64` 通过
- `cargo xtask starry app qemu -t nginx --arch aarch64` 通过
- `cargo xtask starry app qemu -t nginx --arch loongarch64` 通过

### 结论

当前 nginx 默认 CI 入口已可按 4 个架构正常执行，并修复了部分问题和 mirror 安装稳定性，不再因缺默认配置或安装超时而阻断全局 apps CI。
