# Assets

Simple layout for this small project:

- `downloads/`: files downloaded by following `www/proj57/proj57` instructions.
- `prepare/`: runtime-ready assets.

Prepare layout (flat):

- `prepare/model.onnx`
- `prepare/model.onnx.data` (external tensor data file)
- `prepare/stats.json`
- `prepare/input_state.bin` (optional)
- `prepare/input.jpg` (golden image)
- `prepare/golden.json`
- `prepare/review_<case>.jpg`
- `prepare/export_onnx.py`
- `prepare/generate_golden.py`
- `prepare/verify_parity.py`

Default asset root is `assets/prepare`.

Notes:

- `input_state.bin` stores raw `[left_vel, right_vel]` values.
- `generate_golden.py` and `verify_parity.py` normalize state with
  `stats.json` (`observation.state.q01/q99`) before model inference,
  following Task 3 rules in `www/proj57/proj57/README.md`.
