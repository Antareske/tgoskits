#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd "$script_dir/.." && pwd)"
manifest="$app_dir/act-infer/Cargo.toml"
out_dir="$app_dir/output/linux"

mkdir -p "$out_dir"
cargo build --release --manifest-path "$manifest" --bin act-infer-golden-tract --bin act-infer-review-tract --bin act-infer-golden-ort --bin act-infer-review-ort

install -Dm0755 "$app_dir/act-infer/target/release/act-infer-golden-tract" "$out_dir/act-infer-golden-tract"
install -Dm0755 "$app_dir/act-infer/target/release/act-infer-review-tract" "$out_dir/act-infer-review-tract"
install -Dm0755 "$app_dir/act-infer/target/release/act-infer-golden-ort" "$out_dir/act-infer-golden-ort"
install -Dm0755 "$app_dir/act-infer/target/release/act-infer-review-ort" "$out_dir/act-infer-review-ort"

echo "built linux binaries in $out_dir"
