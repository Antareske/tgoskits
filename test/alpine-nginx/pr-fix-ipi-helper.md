# fix(somehal): send x86 helper IPI on IPI vector

## 问题

x86_64 dynamic platform 的 `IPI_IRQ` 定义为 `0xf3`，runtime 也用该 IRQ 注册 `ipi_irq_handler`。但 `somehal` 的 x86 `send_ipi_to_cpu()` helper 错误发送了 LAPIC timer vector `0x20`，导致 helper 路径的 IPI 会被分派为 systimer IRQ，而不是进入 IPI handler。

## 修改

- 在 `platforms/somehal/src/arch/x86_64/mod.rs` 增加 `APIC_IPI_VECTOR = 0xf3`。
- 将 `send_ipi_to_cpu()` 从发送 `APIC_TIMER_VECTOR` 改为发送 `APIC_IPI_VECTOR`。
- 保持 timer vector `0x20` 的原有用途不变。

## 影响

- 修复 x86 helper-based IPI 的分派语义。
- 避免 IPI 被误当作 LAPIC timer interrupt 处理。
- 不影响正常 timer IRQ 路径。

## 验证

- `cargo fmt`
- `cargo xtask clippy --package somehal`
