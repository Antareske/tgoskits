#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import numpy as np
import onnxruntime as ort
from PIL import Image
import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Verify parity between PyTorch and ONNX")
    parser.add_argument("--proj57-root", required=True, help="Absolute path to www/proj57/proj57")
    parser.add_argument("--model-pt", required=True, help="Absolute path to model.pt")
    parser.add_argument("--model-onnx", required=True, help="Absolute path to model.onnx")
    parser.add_argument("--stats", required=True, help="Absolute path to stats.json")
    parser.add_argument("--image", required=True, help="Absolute path to input jpg")
    parser.add_argument("--state-bin", required=True, help="Absolute path to input_state.bin")
    parser.add_argument("--atol", type=float, default=1e-4)
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


def main() -> None:
    args = parse_args()
    proj57_root = Path(args.proj57_root)
    model_pt = Path(args.model_pt)
    model_onnx = Path(args.model_onnx)
    stats_path = Path(args.stats)
    image_path = Path(args.image)
    state_path = Path(args.state_bin)
    for p in [proj57_root, model_pt, model_onnx, stats_path, image_path, state_path]:
        if not p.is_absolute():
            raise SystemExit(f"path must be absolute: {p}")

    sys.path.insert(0, str(proj57_root))
    from act.defaults import build_act_config
    from act.modeling_act import ACTModel

    checkpoint = torch.load(model_pt, map_location="cpu")
    config = build_act_config(**checkpoint["config"])
    model = ACTModel(config)
    model.load_state_dict(checkpoint["model_state_dict"], strict=True)
    model.eval()

    image = preprocess_image(image_path)
    stats = json.loads(stats_path.read_text())
    state = normalize_state(read_state(state_path), stats)

    image_t = torch.from_numpy(image).unsqueeze(0)
    state_t = torch.from_numpy(state).unsqueeze(0)
    with torch.no_grad():
        out_pt = model(image_t, state_t, action_target=None, infer_cvae=False)["action"].cpu().numpy()

    sess = ort.InferenceSession(str(model_onnx), providers=["CPUExecutionProvider"])
    out_onnx = sess.run(None, {"image": np.expand_dims(image, 0), "state": np.expand_dims(state, 0)})[0]

    max_abs_diff = float(np.max(np.abs(out_pt - out_onnx)))
    print(f"max_abs_diff={max_abs_diff:.8f}")
    print(f"atol={args.atol:.8f}")
    if max_abs_diff > args.atol:
        raise SystemExit(f"parity failed: {max_abs_diff} > {args.atol}")
    print("parity_ok")


if __name__ == "__main__":
    main()
