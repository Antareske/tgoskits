cargo xtask starry test qemu --target riscv64gc-unknown-none-elf -c syscall

cargo xtask starry test qemu --target aarch64-unknown-none-softfloat -c syscall

cargo xtask starry test qemu --target x86_64-unknown-none -c syscall

cargo xtask starry test qemu --target loongarch64-unknown-none-softfloat -c syscall