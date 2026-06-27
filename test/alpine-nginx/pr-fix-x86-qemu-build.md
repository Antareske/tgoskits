# fix(starry): route app qemu through dynamic boot

## 背景

Starry app 的 `qemu` 路径在 x86_64 上会直接把 dynamic 平台内核以 `-kernel <ELF>` 方式交给 QEMU。当前生成的 ELF 不包含 PVH ELF Note，因此会在启动阶段失败：

```text
qemu-system-x86_64: Error loading uncompressed kernel without PVH ELF Note
```

该问题不属于 nginx 业务本身，而是 `scripts/axbuild` 的 app-qemu 派发路径没有复用 dynamic platform boot 逻辑。

## 修改内容

1. 在 `scripts/axbuild/src/context/mod.rs` 中，将 `Context::qemu()` 的 x86 KVM 预处理替换为 `apply_dynamic_platform_qemu_boot()`。
2. 删除 `scripts/axbuild/src/test/qemu.rs` 中已无调用方的 `apply_x86_64_kvm_accel_if_available()` 包装函数。

## 原因

`apply_dynamic_platform_qemu_boot()` 已经封装了 dynamic 平台所需的 QEMU 修正：

- x86_64 的 `uefi=true`
- `to_bin=true`
- 额外的 x86_64 QEMU 参数调整

这和 `arceos` / `starry test` 等既有路径保持一致。把它补到 `Context::qemu()` 后，`starry app qemu` 会自动走已验证的 UEFI/BIN 启动路径，而不是继续触发裸 ELF + PVH 失败。

## 验证

- `cargo fmt --all`
- `cargo xtask clippy --package axbuild`
- `cargo xtask starry app qemu -t redis --arch x86_64`

验证结果显示 x86_64 app-qemu 已切换为 OVMF + pflash + `starryos.esp`，不再使用裸 `-kernel` 直启。

## 影响范围

- 仅影响 `starry app qemu` 这条原先漏掉的 dynamic boot 路径。
- 不影响已在上层显式调用 `apply_dynamic_platform_qemu_boot()` 的 ArceOS / Starry test 路径。
- 非 dynamic target 保持原有行为不变。
