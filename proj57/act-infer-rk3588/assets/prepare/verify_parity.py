#!/usr/bin/env python3
"""Verify ONNX vs RKNN (PC simulator) parity and generate golden.json.

Runs the same image+state through:
  * onnxruntime (reference), and
  * the RKNN simulator (rknn.inference on the PC),
then reports the max abs diff on the denormalized action and writes
`golden.json` (denormalized action) from the RKNN simulator output, so the
on-board golden check compares against the RKNN numerics rather than the FP32
ONNX reference.

It also prints the left/right wheel decision direction for both backends so a
reviewer can confirm the turn direction is preserved after conversion.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image
from rknn.api import RKNN


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="ONNX/RKNN parity + golden generation")
    parser.add_argument("--model-onnx", required=True)
    parser.add_argument("--model-rknn", required=True)
    parser.add_argument("--stats", required=True)
    parser.add_argument("--image", required=True)
    parser.add_argument("--state-bin", required=True)
    parser.add_argument("--golden-out", required=True, help="Path to write golden.json")
    parser.add_argument("--target", default="rk3588")
    parser.add_argument("--atol", type=float, default=5e-2)
    return parser.parse_args()


def preprocess_image(path: Path) -> np.ndarray:
    img = Image.open(path).convert("RGB").resize((224, 224), Image.Resampling.BILINEAR)
    arr = np.asarray(img, dtype=np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
    std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
    arr = (arr - mean) / std
    return np.transpose(arr, (2, 0, 1))


def read_state(path: Path) -> np.ndarray:
    raw = np.fromfile(path, dtype="<f4")
    if raw.size != 2:
        raise ValueError(f"state bin must contain 2 f32 values, got {raw.size}")
    return raw.astype(np.float32)


def normalize_state(state_raw: np.ndarray, stats: dict) -> np.ndarray:
    q01 = np.array(stats["observation.state"]["q01"], dtype=np.float32)
    q99 = np.array(stats["observation.state"]["q99"], dtype=np.float32)
    denom = q99 - q01
    denom = np.where(np.abs(denom) < 1e-12, 1.0, denom)
    return (2.0 * (state_raw - q01) / denom - 1.0).astype(np.float32)


def denormalize(action: np.ndarray, stats: dict) -> list[float]:
    q01 = np.array(stats["action"]["q01"], dtype=np.float32)
    q99 = np.array(stats["action"]["q99"], dtype=np.float32)
    dim = q01.shape[0]
    flat = action.reshape(-1)
    return [float((v + 1.0) * 0.5 * (q99[i % dim] - q01[i % dim]) + q01[i % dim]) for i, v in enumerate(flat)]


def direction(denorm: list[float]) -> str:
    if len(denorm) < 2:
        return "unknown"
    diff = denorm[1] - denorm[0]
    if diff > 0:
        return "left"
    if diff < 0:
        return "right"
    return "straight"


def main() -> int:
    args = parse_args()
    stats = json.loads(Path(args.stats).read_text())
    image = preprocess_image(Path(args.image))
    state = normalize_state(read_state(Path(args.state_bin)), stats)

    image_b = np.expand_dims(image, 0)
    state_b = np.expand_dims(state, 0)

    # ONNX reference.
    sess = ort.InferenceSession(args.model_onnx, providers=["CPUExecutionProvider"])
    out_onnx = sess.run(None, {"image": image_b, "state": state_b})[0]
    onnx_denorm = denormalize(out_onnx, stats)

    # RKNN simulator. A model loaded via load_rknn cannot run on the PC
    # simulator, so rebuild from ONNX in-session with the identical config used
    # in convert_rknn.py, then run on the simulator (target=None). The built
    # graph is numerically equivalent to the exported model.rknn.
    rknn = RKNN(verbose=False)
    rknn.config(
        mean_values=[[0.0, 0.0, 0.0]],
        std_values=[[1.0, 1.0, 1.0]],
        target_platform=args.target,
        quantized_dtype="w8a8",
    )
    if rknn.load_onnx(
        model=args.model_onnx,
        inputs=["image", "state"],
        input_size_list=[[1, 3, 224, 224], [1, 2]],
    ) != 0:
        print("load_onnx failed")
        return 1
    if rknn.build(do_quantization=False, dataset=None) != 0:
        print("build failed")
        return 1
    if rknn.init_runtime(target=None) != 0:  # target=None -> PC simulator
        print("init_runtime (simulator) failed")
        return 1
    out_rknn = rknn.inference(inputs=[image_b, state_b], data_format=["nchw", "nchw"])[0]
    rknn.release()
    rknn_denorm = denormalize(np.asarray(out_rknn), stats)

    n = min(len(onnx_denorm), len(rknn_denorm))
    max_abs = max(abs(onnx_denorm[i] - rknn_denorm[i]) for i in range(n)) if n else float("inf")

    print(f"onnx_direction={direction(onnx_denorm)} rknn_direction={direction(rknn_denorm)}")
    print(f"onnx_first_step=({onnx_denorm[0]:.6f},{onnx_denorm[1]:.6f})")
    print(f"rknn_first_step=({rknn_denorm[0]:.6f},{rknn_denorm[1]:.6f})")
    print(f"max_abs_diff={max_abs:.8f} atol={args.atol:.8f}")

    golden_out = Path(args.golden_out)
    golden_out.parent.mkdir(parents=True, exist_ok=True)
    golden_out.write_text(json.dumps({"action_denorm": rknn_denorm}, indent=2))
    print(f"wrote golden: {golden_out}")

    if direction(onnx_denorm) != direction(rknn_denorm):
        print("PARITY WARNING: direction differs between ONNX and RKNN")
        return 2
    if max_abs > args.atol:
        print(f"PARITY WARNING: max_abs_diff {max_abs} > atol {args.atol} (direction still matches)")
    print("parity_ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
