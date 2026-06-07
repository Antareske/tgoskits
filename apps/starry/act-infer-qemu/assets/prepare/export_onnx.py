#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path
import sys

import numpy as np
import onnx
from PIL import Image
import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export deterministic ACT ONNX model")
    parser.add_argument(
        "--proj57-root",
        help="Absolute path to proj57 source root; default resolves from app third_party clone",
    )
    parser.add_argument("--model-pt", required=True, help="Absolute path to downloaded model.pt")
    parser.add_argument("--output-onnx", required=True, help="Absolute path to output model.onnx")
    parser.add_argument("--sample-image", required=True, help="Absolute path to sample jpg for tracing")
    parser.add_argument("--state", default="0.0,0.0", help="Raw state as left,right")
    parser.add_argument("--opset", type=int, default=17)
    return parser.parse_args()


def resolve_proj57_root(raw: str | None, script_dir: Path) -> Path:
    if raw:
        root = Path(raw)
    else:
        app_dir = script_dir.parent.parent
        candidates = [
            app_dir / "third_party" / "proj57" / "proj57",
            app_dir / "third_party" / "proj57",
        ]
        root = next((p for p in candidates if (p / "act").is_dir()), candidates[0])
    if not root.is_absolute():
        raise SystemExit(f"path must be absolute: {root}")
    if not (root / "act").is_dir():
        raise SystemExit(f"proj57 root missing act package: {root}")
    return root


def parse_state(raw: str) -> np.ndarray:
    parts = [p.strip() for p in raw.split(",")]
    if len(parts) != 2:
        raise ValueError(f"invalid --state {raw}, expected left,right")
    return np.array([float(parts[0]), float(parts[1])], dtype=np.float32)


def preprocess_image(path: Path) -> np.ndarray:
    img = Image.open(path).convert("RGB").resize((224, 224), Image.Resampling.BILINEAR)
    arr = np.asarray(img, dtype=np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
    std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
    arr = (arr - mean) / std
    return np.transpose(arr, (2, 0, 1))


def main() -> None:
    args = parse_args()
    script_dir = Path(__file__).resolve().parent
    proj57_root = resolve_proj57_root(args.proj57_root, script_dir)
    model_pt = Path(args.model_pt)
    output_onnx = Path(args.output_onnx)
    sample_image = Path(args.sample_image)
    for p in [model_pt, sample_image]:
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

    inference_latent_mu = checkpoint.get("inference_latent_mu")
    if inference_latent_mu is None:
        raise SystemExit("checkpoint missing inference_latent_mu")
    inference_latent_mu = inference_latent_mu.to(dtype=torch.float32).reshape(1, -1)

    class ExportWrapper(torch.nn.Module):
        def __init__(self, inner: torch.nn.Module, latent_mu: torch.Tensor):
            super().__init__()
            self.inner = inner
            self.register_buffer("latent_mu", latent_mu)

        def forward(self, image: torch.Tensor, state: torch.Tensor) -> torch.Tensor:
            batch_size = image.shape[0]
            latent = self.latent_mu.to(image.device)
            if batch_size > 1:
                latent = latent.expand(batch_size, -1)

            vision_features = self.inner.vision_encoder(image)
            state_features = self.inner.state_encoder(state)
            latent_features = self.inner.latent_proj(latent).unsqueeze(1)

            encoder_in = torch.cat([latent_features, state_features, vision_features], dim=1)
            seq_len = encoder_in.shape[1]
            if seq_len <= self.inner.encoder_pos_embed.num_embeddings:
                pos_embed = self.inner.encoder_pos_embed.weight[:seq_len].unsqueeze(0)
            else:
                repeat_count = (seq_len // self.inner.encoder_pos_embed.num_embeddings) + 1
                pos_embed = self.inner.encoder_pos_embed.weight.repeat(1, repeat_count, 1)[:, :seq_len]

            encoder_out = self.inner.encoder(encoder_in, pos_embed=pos_embed)

            decoder_pos_embed = self.inner.decoder_pos_embed.weight.unsqueeze(0).expand(batch_size, -1, -1)
            decoder_in = torch.zeros(
                batch_size,
                self.inner.config.action_chunk_size,
                self.inner.config.hidden_dim,
                device=image.device,
            ) + decoder_pos_embed

            decoder_out = self.inner.decoder(
                decoder_in,
                encoder_out,
                decoder_pos_embed=decoder_pos_embed,
                encoder_pos_embed=pos_embed,
            )

            return self.inner.action_head(decoder_out)

    wrapper = ExportWrapper(model, inference_latent_mu).eval()
    image = torch.from_numpy(preprocess_image(sample_image)).unsqueeze(0)
    state = torch.from_numpy(parse_state(args.state)).unsqueeze(0)

    output_onnx.parent.mkdir(parents=True, exist_ok=True)
    with torch.no_grad():
        torch.onnx.export(
            wrapper,
            (image, state),
            output_onnx,
            export_params=True,
            opset_version=args.opset,
            do_constant_folding=True,
            input_names=["image", "state"],
            output_names=["action"],
            dynamo=False,
        )
    merged = onnx.load(str(output_onnx), load_external_data=True)
    onnx.save_model(merged, str(output_onnx), save_as_external_data=False)

    external_data = output_onnx.with_name(output_onnx.name + ".data")
    if external_data.exists():
        external_data.unlink()
    print(f"exported onnx: {output_onnx}")


if __name__ == "__main__":
    main()
