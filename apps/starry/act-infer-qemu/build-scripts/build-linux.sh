#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd "$script_dir/.." && pwd)"
manifest="$app_dir/act-infer/Cargo.toml"
out_dir="$app_dir/output/linux"

mkdir -p "$out_dir"
cargo build --release --manifest-path "$manifest" --bin act-infer-golden --bin act-infer-review

install -Dm0755 "$app_dir/act-infer/target/release/act-infer-golden" "$out_dir/act-infer-golden"
install -Dm0755 "$app_dir/act-infer/target/release/act-infer-review" "$out_dir/act-infer-review"

echo "built linux binaries in $out_dir"
