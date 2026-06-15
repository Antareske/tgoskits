#!/usr/bin/env bash
set -euo pipefail

app_dir="${STARRY_APP_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
overlay_dir="${STARRY_OVERLAY_DIR:-}"
assets_dir="${ACT_ASSETS_DIR:-$app_dir/assets/prepare}"

require_env() {
    local name="$1"
    local value="$2"
    if [[ -z "$value" ]]; then
        echo "error: $name is required" >&2
        exit 1
    fi
}

ensure_assets() {
    local required=(
        "$assets_dir/model.onnx"
        "$assets_dir/stats.json"
        "$assets_dir/golden.json"
        "$assets_dir/input.jpg"
    )
    local missing=()
    local path
    for path in "${required[@]}"; do
        [[ -f "$path" ]] || missing+=("$path")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        echo "error: missing prepared assets:" >&2
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
    cargo build --release --manifest-path "$manifest" --bin act-infer-golden-tract --bin act-infer-review-tract

    local linker
    linker="$(find_musl_linker)"
    CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER="$linker" \
        cargo build --release --target riscv64gc-unknown-linux-musl --manifest-path "$manifest" --bin act-infer-golden-tract --bin act-infer-review-tract
}

copy_overlay() {
    install -Dm0755 "$app_dir/act-infer-golden.sh" "$overlay_dir/usr/bin/act-infer-golden.sh"
    install -Dm0755 "$app_dir/act-infer-review.sh" "$overlay_dir/usr/bin/act-infer-review.sh"
    install -Dm0755 \
        "$app_dir/act-infer/target/riscv64gc-unknown-linux-musl/release/act-infer-golden-tract" \
        "$overlay_dir/usr/bin/act_infer_golden_tract"
    install -Dm0755 \
        "$app_dir/act-infer/target/riscv64gc-unknown-linux-musl/release/act-infer-review-tract" \
        "$overlay_dir/usr/bin/act_infer_review_tract"

    install -Dm0644 "$assets_dir/model.onnx" "$overlay_dir/opt/act/model.onnx"
    install -Dm0644 "$assets_dir/stats.json" "$overlay_dir/opt/act/stats.json"
    install -Dm0644 "$assets_dir/golden.json" "$overlay_dir/opt/act/golden.json"
    install -Dm0644 "$assets_dir/input.jpg" "$overlay_dir/opt/act/input.jpg"
    if [[ -f "$assets_dir/input_state.bin" ]]; then
        install -Dm0644 "$assets_dir/input_state.bin" "$overlay_dir/opt/act/input_state.bin"
    fi
    if [[ -f "$assets_dir/review_meta.env" ]]; then
        install -Dm0644 "$assets_dir/review_meta.env" "$overlay_dir/opt/act/review_meta.env"
    fi
}

require_env STARRY_OVERLAY_DIR "$overlay_dir"

ensure_assets
build_infer_bins
copy_overlay
