#!/usr/bin/env bash
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace="$(cd "$app_dir/../../.." && pwd)"

if [[ $# -lt 2 ]]; then
    echo "usage: $0 <case_name> <expected_direction>" >&2
    echo "example: $0 left left" >&2
    exit 1
fi

case_name="$1"
expected_direction="$2"
assets_dir="${ACT_ASSETS_DIR:-$app_dir/assets/prepare}"
case_image="$assets_dir/review_${case_name}.jpg"
if [[ ! -f "$case_image" ]]; then
    echo "error: case image not found: $case_image" >&2
    exit 1
fi

deploy_dir="$app_dir/assets/prepare/runtime-tmp"
mkdir -p "$deploy_dir"

for file in "$assets_dir/model.onnx" "$assets_dir/stats.json" "$case_image"; do
    if [[ ! -f "$file" ]]; then
        echo "error: missing required file $file" >&2
        exit 1
    fi
done

cp "$assets_dir/model.onnx" "$deploy_dir/model.onnx"
cp "$assets_dir/stats.json" "$deploy_dir/stats.json"
cp "$case_image" "$deploy_dir/input.jpg"

if [[ -f "$assets_dir/input_state.bin" ]]; then
    cp "$assets_dir/input_state.bin" "$deploy_dir/input_state.bin"
fi

if [[ -f "$assets_dir/golden.json" ]]; then
    cp "$assets_dir/golden.json" "$deploy_dir/golden.json"
fi

cat > "$deploy_dir/review_meta.env" <<EOF
ACT_REVIEW_CASE=$case_name
ACT_REVIEW_EXPECTED=$expected_direction
EOF

mkdir -p "$workspace/tmp/act-infer-review"
stamp="$(date +%Y%m%d-%H%M%S)"
log_file="$workspace/tmp/act-infer-review/${case_name}-${stamp}.log"

echo "info: running review case=$case_name expected=$expected_direction"
echo "info: log will be saved to $log_file"

env -u LD_PRELOAD \
    ACT_ASSETS_DIR="$deploy_dir" \
    cargo xtask starry app run -t act-infer-qemu --arch riscv64 --qemu-config "$app_dir/qemu-riscv64-review.toml" \
    | tee "$log_file"

echo "info: review log saved at $log_file"
