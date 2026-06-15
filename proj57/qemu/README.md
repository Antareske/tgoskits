# proj57/qemu — ACT 模型 QEMU/StarryOS CPU 推理（任务三）

## 第一部分：推理程序与资产准备

本目录实现了一套轻量 ACT 推理流程，包含两组 Rust 可执行程序：

- `act-infer-golden-tract` / `act-infer-review-tract`：使用 `tract` 运行 ONNX，面向 Starry guest 链路。
- `act-infer-golden-ort` / `act-infer-review-ort`：使用 ONNX Runtime `ort`，面向宿主机对照验证。

四个程序都支持：

- 使用绝对路径参数；
- 将结构化 JSON 输出到 stdout；
- 通过 `--output` 可选落盘同样的 JSON。

### 目录结构

- `act-infer/`：Rust 源码（`src/bin/*_tract.rs`、`src/bin/*_ort.rs` 与共享推理逻辑）。
- `assets/prepare/`：准备好的运行时资产与 Python 工具脚本。
- `setup-python-env.sh`：创建 `.venv`、安装 `requirements.txt`、克隆 proj57 赛题仓库 `proj57`。

### 导出与一致性校验流程

`assets/prepare/` 下常用脚本：

- `export_onnx.py`：从 proj57 checkpoint 导出 ONNX。
- `verify_parity.py`：校验 ONNX 与源推理流程行为一致性。
- `generate_golden.py`：生成/更新 `golden.json`。

默认输入资产：

- `model.onnx`
- `stats.json`
- `input.jpg`
- `golden.json`（golden 模式必需）
- `input_state.bin`（可选）

规则说明：

- `input_state.bin` 按原始 `[left_vel, right_vel]` 读取，再由 `stats.json` 做归一化。
- 模型输出为归一化动作块，JSON 中同时给出归一化与反归一化结果。
- ONNX 导出时 latent 固定为 checkpoint 的 `inference_latent_mu`，运行时不接收 latent 输入。
- `tract` 和 `ort` 入口采用统一后缀命名：`*-tract` / `*-ort`。
- Starry guest 当前使用 `tract` 二进制；`ort` 只在非 `riscv64` 宿主机目标编译。

## 第二部分：tgoskits Starry 测试流程

### 两个直接测试入口

1) golden 入口：

```bash
bash proj57/qemu/run-golden.sh
```

2) review 入口：

```bash
bash proj57/qemu/review-run.sh <case_name> <expected_direction>
# 示例
bash proj57/qemu/review-run.sh left left
```

### `cargo xtask starry app run` 执行链路

> 注意：`cargo xtask starry app run` 仅在 `apps/starry/<case>` 下发现 app。本目录已统一到
> `proj57/qemu`，不再常驻 `apps/starry/`。`run-golden.sh` / `review-run.sh` 会在运行时
> 临时创建软链接 `apps/starry/act-infer-qemu -> proj57/qemu`（已被 `.gitignore` 忽略，
> 脚本退出时自动清理），供 xtask 发现该 app。手动执行 `cargo xtask starry app run -t
> act-infer-qemu` 前需自行建立同名软链接。

1. `xtask` 发现该 app，并在存在时执行 `prebuild.sh`。
2. `prebuild.sh` 校验资产，编译 `act-infer-golden-tract`/`act-infer-review-tract`（`riscv64gc-unknown-linux-musl`），并将文件放入 overlay。
3. overlay 注入 app rootfs。
4. QEMU 启动后执行对应 `shell_init_cmd`。
5. 最终根据 `success_regex`/`fail_regex` 判定通过或失败。

### 涉及的关键文件

- `prebuild.sh`：编译并打包二进制、脚本和资产到 overlay。
- `qemu-riscv64.toml`：golden 配置（`shell_init_cmd=/usr/bin/act-infer-golden.sh`）。
- `qemu-riscv64-review.toml`：review 配置（`shell_init_cmd=/usr/bin/act-infer-review.sh`）。
- `act-infer-golden.sh`：guest 内 golden 执行脚本，调用 `/usr/bin/act_infer_golden_tract`，成功标记为 `ACT_INFER_OK`。
- `act-infer-review.sh`：guest 内 review 执行脚本，调用 `/usr/bin/act_infer_review_tract`，成功标记为 `ACT_REVIEW_DONE`。
- `run-golden.sh`：host 侧 golden 直跑入口。
- `review-run.sh`：host 侧 review 直跑入口（支持 `ACT_ASSETS_DIR` 覆盖资产目录）。

### 宿主机推理二进制构建

```bash
bash proj57/qemu/build-scripts/build-linux.sh
```

该脚本会在 `proj57/qemu/output/linux` 下生成四个二进制：

- `act-infer-golden-tract`
- `act-infer-review-tract`
- `act-infer-golden-ort`
- `act-infer-review-ort`

### 资产目录覆盖

默认读取 `proj57/qemu/assets/prepare`。如需替换为其他数据集：

```bash
ACT_ASSETS_DIR=/abs/path/to/assets/prepare bash proj57/qemu/run-golden.sh
ACT_ASSETS_DIR=/abs/path/to/assets/prepare bash proj57/qemu/review-run.sh right right
```
