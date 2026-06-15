#!/usr/bin/env bash
# Cross-compile the RK3588 ACT inference binaries (aarch64 GNU/Linux) and stage
# a self-contained install directory that can be rsync'd to the board rootfs.
#
# Output layout (install/rk3588_linux_aarch64/act_infer_rk3588/):
#   act-infer-golden-rknn        golden self-check binary
#   act-infer-review-rknn        review (left/right decision) binary
#   lib/librknnrt.so             RKNPU2 runtime (resolved via $ORIGIN/lib rpath)
#   model/model.rknn             converted FP16 RKNN model
#   model/stats.json             QUANTILE normalization stats
#   model/golden.json            golden denormalized action (RKNN simulator)
#   model/input.jpg              default golden input image
#   model/input_state.bin        raw [left_vel,right_vel] state
#   model/review_left.jpg        review case: expected left turn
#   model/review_right.jpg       review case: expected right turn
#   run-golden.sh / run-review.sh on-board convenience launchers
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
crate_dir="${app_dir}/act-infer"
assets_dir="${app_dir}/assets/prepare"
sdk_lib="${app_dir}/assets/sdk/aarch64/librknnrt.so"
target="aarch64-unknown-linux-gnu"
install_dir="${app_dir}/install/rk3588_linux_aarch64/act_infer_rk3588"

if ! command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
    echo "error: aarch64-linux-gnu-gcc not found (install gcc-aarch64-linux-gnu)" >&2
    exit 1
fi
if ! rustup target list --installed 2>/dev/null | grep -qx "${target}"; then
    echo "error: rust target ${target} not installed (rustup target add ${target})" >&2
    exit 1
fi

required_assets=(
    "${assets_dir}/model.rknn"
    "${assets_dir}/stats.json"
    "${assets_dir}/golden.json"
    "${assets_dir}/input.jpg"
    "${sdk_lib}"
)
missing=()
for path in "${required_assets[@]}"; do
    [[ -f "${path}" ]] || missing+=("${path}")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "error: missing required artifacts:" >&2
    printf '  %s\n' "${missing[@]}" >&2
    echo "run scripts/prepare-model.sh first to produce model.rknn + golden.json" >&2
    exit 1
fi

echo "info: building ${target} release binaries"
( cd "${crate_dir}" && cargo build --release --target "${target}" \
    --bin act-infer-golden-rknn --bin act-infer-review-rknn )

bin_dir="${crate_dir}/target/${target}/release"

rm -rf "${install_dir}"
mkdir -p "${install_dir}/lib" "${install_dir}/model"

install -m0755 "${bin_dir}/act-infer-golden-rknn" "${install_dir}/act-infer-golden-rknn"
install -m0755 "${bin_dir}/act-infer-review-rknn" "${install_dir}/act-infer-review-rknn"
install -m0644 "${sdk_lib}" "${install_dir}/lib/librknnrt.so"

install -m0644 "${assets_dir}/model.rknn" "${install_dir}/model/model.rknn"
install -m0644 "${assets_dir}/stats.json" "${install_dir}/model/stats.json"
install -m0644 "${assets_dir}/golden.json" "${install_dir}/model/golden.json"
install -m0644 "${assets_dir}/input.jpg" "${install_dir}/model/input.jpg"
[[ -f "${assets_dir}/input_state.bin" ]] && \
    install -m0644 "${assets_dir}/input_state.bin" "${install_dir}/model/input_state.bin"
[[ -f "${assets_dir}/review_left.jpg" ]] && \
    install -m0644 "${assets_dir}/review_left.jpg" "${install_dir}/model/review_left.jpg"
[[ -f "${assets_dir}/review_right.jpg" ]] && \
    install -m0644 "${assets_dir}/review_right.jpg" "${install_dir}/model/review_right.jpg"

install -m0755 "${app_dir}/scripts/on-board-run-golden.sh" "${install_dir}/run-golden.sh"
install -m0755 "${app_dir}/scripts/on-board-run-review.sh" "${install_dir}/run-review.sh"

echo "info: installed to ${install_dir}"
ls -la "${install_dir}"
