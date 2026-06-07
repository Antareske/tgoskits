#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd "$script_dir/.." && pwd)"
manifest="$app_dir/act-infer/Cargo.toml"
out_dir="$app_dir/output/starry-riscv64"

find_musl_linker() {
    if command -v riscv64-linux-musl-gcc >/dev/null 2>&1; then
        command -v riscv64-linux-musl-gcc
        return 0
    fi
    if [[ -x "/opt/musl/riscv64-linux-musl-cross/bin/riscv64-linux-musl-gcc" ]]; then
        printf '%s\n' "/opt/musl/riscv64-linux-musl-cross/bin/riscv64-linux-musl-gcc"
        return 0
    fi
    return 1
}

mkdir -p "$out_dir"
linker="$(find_musl_linker)"
CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER="$linker" \
    cargo build --release --target riscv64gc-unknown-linux-musl --manifest-path "$manifest" --bin act-infer-golden-tract --bin act-infer-review-tract

install -Dm0755 "$app_dir/act-infer/target/riscv64gc-unknown-linux-musl/release/act-infer-golden-tract" "$out_dir/act-infer-golden-tract"
install -Dm0755 "$app_dir/act-infer/target/riscv64gc-unknown-linux-musl/release/act-infer-review-tract" "$out_dir/act-infer-review-tract"

echo "built starry riscv64 binaries in $out_dir"
