## Summary

- Add C test case support: compile via CMake with musl cross-compiler, inject binary into rootfs via debugfs, run on StarryOS in QEMU
- Add Rust test case support: compile via rustc with musl target (panic=abort, crt-static), same injection flow
- Sample test cases: `helloworld` (C) and `hello-rust` (Rust) under `test-suit/starryos/normal/`
- CI: install `e2fsprogs` in reusable workflow for debugfs support

## Test plan

- [ ] `cargo test -p axbuild` passes
- [ ] `cargo fmt -p axbuild --check` passes
- [ ] `cargo clippy -p axbuild --all-targets --all-features` passes
- [ ] CI `test_qemu_matrix` discovers and runs `smoke`, `helloworld`, `hello-rust` for all 4 archs
- [ ] All test cases pass in QEMU