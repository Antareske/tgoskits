#!/usr/bin/env bash
# End-to-end model preparation for the RK3588 ACT task. Runs entirely inside a
# project-local venv so the global Python environment is never modified.
#
# Stages:
#   1. (optional) download proj57 model.pt from HuggingFace
#   2. export deterministic 2-input ONNX from the checkpoint
#   3. convert ONNX -> RK3588 .rknn (FP16, no quantization by default)
#   4. verify ONNX vs RKNN-simulator parity and (re)generate golden.json
#
# All produced runtime assets land in assets/prepare/.
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
venv="${app_dir}/.venv"
prepare="${app_dir}/assets/prepare"
downloads="${app_dir}/assets/downloads"
proj57_root="${PROJ57_ROOT:-${app_dir}/../../www/proj57}"
proj57_root="$(cd "${proj57_root}" && pwd)"
model_pt="${MODEL_PT:-${downloads}/output/train/model.pt}"

py="${venv}/bin/python"
pip="${venv}/bin/pip"

# 0. venv + dependencies -----------------------------------------------------
if [[ ! -x "${py}" ]]; then
    echo "info: creating venv at ${venv}"
    python3 -m venv "${venv}"
fi
if ! "${py}" -c "import rknn, torch, onnx, onnxruntime" >/dev/null 2>&1; then
    echo "info: installing python dependencies into venv"
    "${pip}" install --upgrade pip
    "${pip}" install --extra-index-url https://download.pytorch.org/whl/cpu \
        "torch==2.4.0+cpu" "torchvision==0.19.0+cpu" \
        "onnx==1.17.0" onnxruntime "numpy<=1.26.4" Pillow "setuptools<81" \
        "${app_dir}/rknn-sdk2/packages/rknn_toolkit2-2.4.2a7-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
fi

# 1. download checkpoint (optional) ------------------------------------------
if [[ ! -f "${model_pt}" ]]; then
    echo "info: model.pt not found, downloading proj57_model from HuggingFace"
    "${pip}" install -q huggingface_hub
    "${py}" - "$downloads" <<'PY'
import sys
from huggingface_hub import snapshot_download
downloads = sys.argv[1]
snapshot_download("bobodai/proj57_model", local_dir=f"{downloads}/output/train")
print("model downloaded")
PY
fi

# 2. export ONNX -------------------------------------------------------------
echo "info: exporting ONNX"
"${py}" "${prepare}/export_onnx.py" \
    --proj57-root "${proj57_root}" \
    --model-pt "${model_pt}" \
    --output-onnx "${prepare}/model.onnx" \
    --sample-image "${prepare}/input.jpg" \
    --state 0.0,0.0

# 3. convert RKNN ------------------------------------------------------------
echo "info: converting ONNX -> RKNN (FP16)"
"${py}" "${prepare}/convert_rknn.py" \
    --input-onnx "${prepare}/model.onnx" \
    --output-rknn "${prepare}/model.rknn" \
    --target rk3588

# 4. parity + golden ---------------------------------------------------------
echo "info: verifying parity and generating golden.json"
"${py}" "${prepare}/verify_parity.py" \
    --model-onnx "${prepare}/model.onnx" \
    --model-rknn "${prepare}/model.rknn" \
    --stats "${prepare}/stats.json" \
    --image "${prepare}/input.jpg" \
    --state-bin "${prepare}/input_state.bin" \
    --golden-out "${prepare}/golden.json"

echo "info: model preparation complete"
ls -la "${prepare}"
