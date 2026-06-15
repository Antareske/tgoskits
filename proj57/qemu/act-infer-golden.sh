#!/bin/sh
set -eu

echo "ACT_INFER_BEGIN"
state_args=""
if [ -f /opt/act/input_state.bin ]; then
    state_args="--state /opt/act/input_state.bin"
fi

if /usr/bin/act_infer_golden_tract \
    --model /opt/act/model.onnx \
    --image /opt/act/input.jpg \
    --normalize /opt/act/stats.json \
    $state_args \
    --golden /opt/act/golden.json \
    --output /tmp/act_golden_result.json; then
    echo "ACT_INFER_OK"
else
    echo "ACT_INFER_FAILED"
    exit 1
fi
