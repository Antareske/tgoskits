#!/usr/bin/env bash
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace="$(cd "$app_dir/../.." && pwd)"
assets_dir="${ACT_ASSETS_DIR:-$app_dir/assets/prepare}"

required=(
    "$assets_dir/model.onnx"
    "$assets_dir/stats.json"
    "$assets_dir/golden.json"
    "$assets_dir/input.jpg"
)

for file in "${required[@]}"; do
    if [[ ! -f "$file" ]]; then
        echo "error: missing required file $file" >&2
        exit 1
    fi
done

# xtask discovers apps under apps/starry/<case>. Provide a symlink if absent.
app_link="$workspace/apps/starry/act-infer-qemu"
if [[ ! -e "$app_link" ]]; then
    ln -s "$app_dir" "$app_link"
    trap 'rm -f "$app_link"' EXIT
fi

mkdir -p "$workspace/tmp/act-infer-golden"
stamp="$(date +%Y%m%d-%H%M%S)"
log_file="$workspace/tmp/act-infer-golden/golden-${stamp}.log"

echo "info: running golden test"
echo "info: assets dir: $assets_dir"
echo "info: log will be saved to $log_file"

env -u LD_PRELOAD \
    ACT_ASSETS_DIR="$assets_dir" \
    cargo xtask starry app run -t act-infer-qemu --arch riscv64 \
    | tee "$log_file"

echo "info: golden log saved at $log_file"
