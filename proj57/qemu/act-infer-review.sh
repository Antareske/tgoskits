#!/bin/sh
set -eu

case_name="unknown"
expected_direction="unknown"

if [ -f /opt/act/review_meta.env ]; then
    # shellcheck disable=SC1091
    . /opt/act/review_meta.env
    case_name="${ACT_REVIEW_CASE:-$case_name}"
    expected_direction="${ACT_REVIEW_EXPECTED:-$expected_direction}"
fi

echo "ACT_REVIEW_CASE=${case_name}"
echo "ACT_REVIEW_EXPECTED=${expected_direction}"
echo "ACT_INFER_BEGIN"

state_args=""
if [ -f /opt/act/input_state.bin ]; then
    state_args="--state /opt/act/input_state.bin"
fi

if /usr/bin/act_infer_review_tract \
    --model /opt/act/model.onnx \
    --image /opt/act/input.jpg \
    --normalize /opt/act/stats.json \
    $state_args \
    --output /tmp/act_review_result.json; then
    echo "ACT_REVIEW_DONE"
else
    echo "ACT_INFER_FAILED"
    exit 1
fi
