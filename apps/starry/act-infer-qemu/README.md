# Starry ACT QEMU App

Refactored layout for two independent user-space inference binaries:

- `act-infer-golden`: accuracy/deviation check against golden labels.
- `act-infer-review`: single-run review output for left/right steering inspection.

Runtime entry scripts in Starry image:

- `act-infer-golden.sh`: default QEMU golden comparison flow (`ACT_INFER_OK` on pass).
- `act-infer-review.sh`: review flow for steering direction inspection.

Both binaries accept absolute paths and always print readable output JSON to stdout.
If `--output` is provided, the same JSON is also written to file.

## Directory Layout

- `act-infer/`: Rust source code for inference binaries.
- `build-scripts/`: Linux and StarryOS riscv64 build/link scripts.
- `output/`: organized build/runtime output directory.
- `assets/`: downloaded source data + prepared runtime assets.

Python preparation helpers:

- `setup-python-env.sh`: create `.venv`, install `assets/prepare/requirements.txt`, clone `git@github.com:chenlongos/proj57.git` into `third_party/proj57`.
- `assets/prepare/export_onnx.py` and `assets/prepare/verify_parity.py` auto-resolve proj57 source from `third_party/proj57` by default.

Default runtime reads from `assets/prepare`:

- `prebuild.sh` packages `assets/prepare` into QEMU overlay.
- `review-run.sh` reads `assets/prepare/review_<case>.jpg`.
- Override with `ACT_ASSETS_DIR=/abs/path/to/assets/prepare`.

## CLI Parameters

Review:

```bash
act-infer-review \
  --model /abs/path/model.onnx \
  --image /abs/path/input.jpg \
  --normalize /abs/path/stats.json \
  [--state /abs/path/input_state.bin] \
  [--output /abs/path/review_result.json]
```

Golden:

```bash
act-infer-golden \
  --model /abs/path/model.onnx \
  --image /abs/path/input.jpg \
  --normalize /abs/path/stats.json \
  --golden /abs/path/golden.json \
  [--state /abs/path/input_state.bin] \
  [--output /abs/path/golden_result.json] \
  [--atol 0.01]
```

## Metrics

Timing metrics only cover model inference execution (`run`) in milliseconds:

- `timing_ms.infer_single_ms`
- `timing_ms.infer_total_ms`
- `timing_ms.run_count`

No asset preparation or postprocessing time is included.

## Rule Alignment Notes

- `input_state.bin` is treated as raw `[left_vel, right_vel]` and normalized by `stats.json`.
- Model raw output is normalized action chunk; JSON outputs include both normalized and denormalized actions.
- Exported ONNX model fixes latent to checkpoint `inference_latent_mu`; runtime does not accept latent input.
