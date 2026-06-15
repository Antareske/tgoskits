#!/bin/sh
# On-board review launcher for StarryOS/Linux on RK3588.
# Runs ACT NPU inference on a chosen case image and prints the decided turn
# direction (left/right). This is the contest-facing behavior check.
#
# Usage: run-review.sh [case]
#   case: left | right | default (default: default -> input.jpg)
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="${here}/lib:${LD_LIBRARY_PATH:-}"

case_name="${1:-default}"
case "${case_name}" in
    left) image="${here}/model/review_left.jpg" ;;
    right) image="${here}/model/review_right.jpg" ;;
    default) image="${here}/model/input.jpg" ;;
    *) echo "unknown case: ${case_name} (expected left|right|default)" >&2; exit 2 ;;
esac

if [ ! -f "${image}" ]; then
    echo "ACT_INFER_FAILED: case image not found: ${image}" >&2
    exit 1
fi

state_args=""
if [ -f "${here}/model/input_state.bin" ]; then
    state_args="--state ${here}/model/input_state.bin"
fi

output_json="${ACT_OUTPUT_JSON:-/tmp/act_review_result.json}"

echo "ACT_REVIEW_CASE=${case_name}"
echo ACT_INFER_BEGIN
# shellcheck disable=SC2086
if "${here}/act-infer-review-rknn" \
    --model "${here}/model/model.rknn" \
    --image "${image}" \
    --normalize "${here}/model/stats.json" \
    $state_args \
    --repeat "${ACT_REPEAT:-1}" \
    --core-mask "${ACT_CORE_MASK:-auto}" \
    --output "${output_json}"; then
    echo ACT_REVIEW_DONE
else
    echo ACT_INFER_FAILED
    exit 1
fi
