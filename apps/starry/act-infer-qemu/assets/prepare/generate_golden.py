#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate golden.json from ONNX output")
    parser.add_argument("--model-onnx", required=True, help="Absolute path to model.onnx")
    parser.add_argument("--stats", required=True, help="Absolute path to stats.json")
    parser.add_argument("--image", required=True, help="Absolute path to input jpg")
    parser.add_argument("--state-bin", required=True, help="Absolute path to input_state.bin")
    parser.add_argument("--output", required=True, help="Absolute path to output golden.json")
    return parser.parse_args()


def preprocess_image(path: Path) -> np.ndarray:
    img = Image.open(path).convert("RGB").resize((224, 224), Image.Resampling.BILINEAR)
    arr = np.asarray(img, dtype=np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
    std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
    arr = (arr - mean) / std
    arr = np.transpose(arr, (2, 0, 1))
    return np.expand_dims(arr, axis=0)


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
    state = 2.0 * (state_raw - q01) / denom - 1.0
    return np.expand_dims(state.astype(np.float32), axis=0)


def denormalize(action: np.ndarray, stats: dict) -> list[float]:
    q01 = np.array(stats["action"]["q01"], dtype=np.float32)
    q99 = np.array(stats["action"]["q99"], dtype=np.float32)
    dim = q01.shape[0]
    flat = action.reshape(-1)
    return [float((v + 1.0) * 0.5 * (q99[i % dim] - q01[i % dim]) + q01[i % dim]) for i, v in enumerate(flat)]


def main() -> None:
    args = parse_args()
    model_path = Path(args.model_onnx)
    stats_path = Path(args.stats)
    image_path = Path(args.image)
    state_path = Path(args.state_bin)
    output_path = Path(args.output)
    for p in [model_path, stats_path, image_path, state_path, output_path]:
        if not p.is_absolute():
            raise SystemExit(f"path must be absolute: {p}")

    stats = json.loads(stats_path.read_text())
    sess = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    image = preprocess_image(image_path)
    state_raw = read_state(state_path)
    state = normalize_state(state_raw, stats)
    out = sess.run(None, {"image": image, "state": state})[0]
    action_denorm = denormalize(out, stats)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps({"action_denorm": action_denorm}, indent=2))
    print(f"generated golden: {output_path}")


if __name__ == "__main__":
    main()
