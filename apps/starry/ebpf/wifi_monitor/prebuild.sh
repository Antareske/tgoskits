#!/usr/bin/env bash
# Build the `wifi_monitor` eBPF probe (aya loader + embedded bytecode) as a
# static musl binary for RISC-V and install it into the StarryOS rootfs overlay.
set -euo pipefail

app_dir="${STARRY_APP_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
overlay_dir="${STARRY_OVERLAY_DIR:?STARRY_OVERLAY_DIR is required}"

musl_target="riscv64gc-unknown-linux-musl"
cross_prefix="riscv64-linux-musl"

cross_bin="/opt/${cross_prefix}-cross/bin"
if [[ -d "$cross_bin" ]]; then
    export PATH="$cross_bin:$PATH"
fi
cc_bin="${cross_prefix}-gcc"

if ! command -v rustup >/dev/null 2>&1; then
    echo "$(basename "$app_dir") prebuild: rustup is required" >&2
    exit 1
fi
read -r rust_toolchain _ < <(rustup show active-toolchain)
export RUSTUP_TOOLCHAIN="$rust_toolchain"

sysroot="$(rustup run "$rust_toolchain" rustc --print sysroot 2>/dev/null)" || true
target_dir="${sysroot:-}/lib/rustlib/${musl_target}"
if [[ ! -d "$target_dir" ]]; then
    echo "$(basename "$app_dir") prebuild: installing Rust target $musl_target"
    rustup target add --toolchain "$rust_toolchain" "$musl_target" || {
        echo "$(basename "$app_dir") prebuild: install failed, trying fallback" >&2
        for tc_dir in "$HOME/.rustup/toolchains/"*; do
            src="${tc_dir}/lib/rustlib/${musl_target}"
            if [[ -d "$src" && "$(basename "$tc_dir")" != "$rust_toolchain" ]]; then
                mkdir -p "$(dirname "$target_dir")"
                cp -r "$src" "$target_dir"
                break
            fi
        done
    }
fi

host_tools_dir="${STARRY_WORKSPACE:-$app_dir}/tmp/axbuild/starry-host-tools"
export PATH="$host_tools_dir/bin:${HOME:-/root}/.cargo/bin:$PATH"

ensure_bpf_linker() {
    if command -v bpf-linker >/dev/null 2>&1; then
        return 0
    fi
    if command -v apk >/dev/null 2>&1; then
        echo "$(basename "$app_dir") prebuild: installing bpf-linker with apk"
        apk add --no-cache bpf-linker || true
        if command -v bpf-linker >/dev/null 2>&1; then
            return 0
        fi
    fi
    echo "$(basename "$app_dir") prebuild: installing bpf-linker with cargo"
    cargo install bpf-linker --version 0.10.3 --locked --root "$host_tools_dir"
}
ensure_bpf_linker

echo "$(basename "$app_dir") prebuild: building eBPF probe for $musl_target (CC=$cc_bin)"
(
    cd "$app_dir"
    export AYA_BPF_TARGET_ARCH=riscv64
    CC="$cc_bin" cargo build --release --target "$musl_target" \
        --config "target.${musl_target}.linker=\"${cc_bin}\""
)

bin="$app_dir/target/$musl_target/release/wifi_monitor"
[[ -x "$bin" ]] || { echo "wifi_monitor prebuild: build did not produce $bin" >&2; exit 1; }

install -Dm0755 "$bin" "$overlay_dir/usr/bin/wifi_monitor"
echo "wifi_monitor prebuild: installed $(basename "$bin") -> /usr/bin/wifi_monitor"
