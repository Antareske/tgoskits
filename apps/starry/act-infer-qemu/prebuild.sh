#!/usr/bin/env bash
set -euo pipefail

app_dir="${STARRY_APP_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
overlay_dir="${STARRY_OVERLAY_DIR:-}"
workspace="${STARRY_WORKSPACE:-$(cd "$app_dir/../../.." && pwd)}"

require_env() {
    local name="$1"
    local value="$2"
    if [[ -z "$value" ]]; then
        echo "error: $name is required" >&2
        exit 1
    fi
}

find_default_act4starry_root() {
    local parent
    parent="$(cd "$workspace/.." && pwd)"
    if [[ -d "$parent/ACT4starry/AKA-Sim2Real" ]]; then
        printf '%s\n' "$parent/ACT4starry/AKA-Sim2Real"
        return 0
    fi
    return 1
}

ensure_assets() {
    local root="$1"
    local deploy_dir="$root/deploy"

    if [[ ! -f "$deploy_dir/model.onnx" ]]; then
        echo "info: model.onnx not found, trying export_act_onnx.py" >&2
        python3 "$root/scripts/export_act_onnx.py"
    fi

    local required=(model.onnx stats.json golden.json input_image.bin input_state.bin)
    local missing=()
    local f
    for f in "${required[@]}"; do
        [[ -f "$deploy_dir/$f" ]] || missing+=("$deploy_dir/$f")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        echo "error: missing ACT deploy assets:" >&2
        printf '  %s\n' "${missing[@]}" >&2
        exit 1
    fi
}

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

build_infer_bins() {
    local manifest="$app_dir/act-infer/Cargo.toml"
    cargo build --release --manifest-path "$manifest"

    local linker
    linker="$(find_musl_linker)"
    CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER="$linker" \
        cargo build --release --target riscv64gc-unknown-linux-musl --manifest-path "$manifest"
}

copy_overlay() {
    local act_root="$1"
    local deploy_dir="$act_root/deploy"

    install -Dm0755 "$app_dir/act-infer-smoke.sh" "$overlay_dir/usr/bin/act-infer-smoke.sh"
    install -Dm0755 \
        "$app_dir/act-infer/target/riscv64gc-unknown-linux-musl/release/act-infer" \
        "$overlay_dir/usr/bin/act_infer"

    install -Dm0644 "$deploy_dir/model.onnx" "$overlay_dir/opt/act/model.onnx"
    install -Dm0644 "$deploy_dir/stats.json" "$overlay_dir/opt/act/stats.json"
    install -Dm0644 "$deploy_dir/golden.json" "$overlay_dir/opt/act/golden.json"
    install -Dm0644 "$deploy_dir/input_image.bin" "$overlay_dir/opt/act/input_image.bin"
    install -Dm0644 "$deploy_dir/input_state.bin" "$overlay_dir/opt/act/input_state.bin"
}

require_env STARRY_OVERLAY_DIR "$overlay_dir"

act4starry_root="${ACT4STARRY_ROOT:-}"
if [[ -z "$act4starry_root" ]]; then
    if act4starry_root="$(find_default_act4starry_root)"; then
        echo "info: ACT4STARRY_ROOT defaulted to $act4starry_root"
    else
        echo "error: ACT4STARRY_ROOT is required when ACT4starry is not next to tgoskits" >&2
        exit 1
    fi
fi

ensure_assets "$act4starry_root"
build_infer_bins
copy_overlay "$act4starry_root"
