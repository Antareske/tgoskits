# Starry ACT QEMU App

This app runs ACT policy inference on StarryOS (QEMU riscv64) as a standalone CI
scenario.

It expects deploy assets from ACT4starry under `${ACT4STARRY_ROOT}/deploy`:

- `model.onnx`
- `stats.json`
- `golden.json`
- `input_image.bin`
- `input_state.bin`

`prebuild.sh` builds two Rust user-space binaries:

- host Linux binary for development-side comparison (`x86_64-unknown-linux-gnu`);
- riscv64 musl static binary for Starry rootfs overlay
  (`riscv64gc-unknown-linux-musl`).

Run:

```bash
env -u LD_PRELOAD cargo xtask starry app run -t act-infer-qemu --arch riscv64
```

By default, `ACT4STARRY_ROOT` is discovered as a sibling path
`../ACT4starry/AKA-Sim2Real` relative to this repository.
