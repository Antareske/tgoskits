#!/bin/sh
# On-board golden self-check launcher for StarryOS/Linux on RK3588.
# Compares NPU inference output against the precomputed golden action.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="${here}/lib:${LD_LIBRARY_PATH:-}"

state_args=""
if [ -f "${here}/model/input_state.bin" ]; then
    state_args="--state ${here}/model/input_state.bin"
fi

output_json="${ACT_OUTPUT_JSON:-/tmp/act_golden_result.json}"

echo ACT_INFER_BEGIN
# shellcheck disable=SC2086
if "${here}/act-infer-golden-rknn" \
    --model "${here}/model/model.rknn" \
    --image "${here}/model/input.jpg" \
    --normalize "${here}/model/stats.json" \
    $state_args \
    --golden "${here}/model/golden.json" \
    --atol "${ACT_ATOL:-0.05}" \
    --repeat "${ACT_REPEAT:-1}" \
    --core-mask "${ACT_CORE_MASK:-auto}" \
    --output "${output_json}"; then
    echo ACT_INFER_OK
else
    echo ACT_INFER_FAILED
    exit 1
fi
