#!/usr/bin/env python3
"""Convert the deterministic ACT ONNX model to an RK3588 .rknn model.

Design choices (see proj57/rk3588/README.md for rationale):

* No quantization (FP16). The RK3588 board has ample memory (4/8GB), so we
  keep the model in float to preserve the decision direction (left/right turn)
  with minimal numerical drift versus the ONNX/PyTorch reference. INT8 would
  shrink/accelerate further but risks sign flips through the CVAE+Transformer.
* The Rust runtime feeds already-normalized float32 NCHW image data and
  normalized float32 state, so RKNN-side mean/std normalization is identity
  (mean=0, std=1). This keeps the preprocessing identical to the QEMU pipeline
  and the parity/golden tooling.
* `target_platform=rk3588`.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

from rknn.api import RKNN


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Convert ACT ONNX to RK3588 RKNN")
    parser.add_argument("--input-onnx", required=True, help="Absolute path to model.onnx")
    parser.add_argument("--output-rknn", required=True, help="Absolute path to output model.rknn")
    parser.add_argument("--target", default="rk3588", help="Target platform")
    parser.add_argument(
        "--quantize",
        action="store_true",
        help="Enable INT8 quantization (requires --dataset). Default is FP16.",
    )
    parser.add_argument(
        "--dataset",
        default=None,
        help="Optional dataset txt for INT8 quantization calibration",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    input_onnx = Path(args.input_onnx)
    output_rknn = Path(args.output_rknn)
    for p in [input_onnx, output_rknn]:
        if not p.is_absolute():
            raise SystemExit(f"path must be absolute: {p}")
    if not input_onnx.is_file():
        raise SystemExit(f"input onnx not found: {input_onnx}")

    rknn = RKNN(verbose=True)

    # Identity normalization: the Rust runtime already produces normalized
    # float32 inputs, so the NPU graph must not re-normalize.
    rknn.config(
        mean_values=[[0.0, 0.0, 0.0]],
        std_values=[[1.0, 1.0, 1.0]],
        target_platform=args.target,
        # Keep float; only quantize when explicitly requested.
        quantized_dtype="w8a8",
    )

    ret = rknn.load_onnx(
        model=str(input_onnx),
        inputs=["image", "state"],
        input_size_list=[[1, 3, 224, 224], [1, 2]],
    )
    if ret != 0:
        print(f"load_onnx failed: ret={ret}", file=sys.stderr)
        return 1

    do_quant = bool(args.quantize)
    if do_quant and not args.dataset:
        print("INT8 quantization requested but --dataset missing", file=sys.stderr)
        return 1

    ret = rknn.build(do_quantization=do_quant, dataset=args.dataset)
    if ret != 0:
        print(f"build failed: ret={ret}", file=sys.stderr)
        return 1

    output_rknn.parent.mkdir(parents=True, exist_ok=True)
    ret = rknn.export_rknn(str(output_rknn))
    if ret != 0:
        print(f"export_rknn failed: ret={ret}", file=sys.stderr)
        return 1

    rknn.release()
    print(f"exported rknn: {output_rknn} (quantized={do_quant})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
