# 任务三（QEMU ACT 推理）迭代测试记录

日期：2026-05-28

## 目标

- 在 `apps/starry/act-infer-qemu/` 形态下迭代验证任务三。
- 不修改系统级全局配置；Python 相关操作仅使用独立 venv。
- 验证主机态推理程序与 StarryOS + QEMU 端到端链路。

## 本次测试环境与约束

- 工作区：`/workspace/tgoskits`
- ACT 资产根目录：`/workspace/tgoskits/www/proj57/AKA-Sim2Real`
- Python venv（初次最小链路）：`/tmp/opencode/act-task3-venv`
- Python venv（真实 output 复测）：`/tmp/opencode/act-real-venv`

## output 目录理解（按 sim2real 文档 + 实际目录）

结合 `docs/src/SimToReal/training.md` 和 `docs/src/SimToReal/data-collection.md`，你当前数据结构为：

```text
output/
├── train/user_1779552434706_capaltfw6/default/model.pt
└── dataset/user_1779552434706_capaltfw6/default/
    ├── data/chunk-*/file-*.parquet
    ├── videos/observation.images.fpv/chunk-*/frame_*.jpg
    └── meta/
        ├── info.json
        ├── stats.json
        └── episodes/chunk-*/episodes.parquet
```

关键含义：

- `model.pt`：训练得到的 ACT checkpoint（包含模型参数与推理相关配置）。
- `meta/stats.json`：状态与动作的 quantile 统计（`q01/q99`），用于归一化与反归一化。
- `data/*.parquet`：每帧的 `observation.image`、`observation.state`、`action`。
- `videos/.../frame_*.jpg`：与 parquet 行对应的图像帧。

本次实际使用样本（`file-000.parquet` 第 0 行）：

- `observation.image = videos/observation.images.fpv/chunk-000/frame_000000.jpg`
- `observation.state = [0.0, 0.0]`

## 执行记录

### 1) 初次最小链路打通（已完成）

初次为了先验证 app 链路，先用最小 mock ONNX 资产打通了完整流程；当时已验证可输出 `ACT_INFER_OK`。

### 2) 切换为真实 output 资产并生成 deploy 文件

在隔离 venv 中安装依赖并执行导出脚本（一次性 Python 命令）：

- 输入：
  - `output/train/user_1779552434706_capaltfw6/default/model.pt`
  - `output/dataset/user_1779552434706_capaltfw6/default/meta/stats.json`
  - `output/dataset/.../data/chunk-000/file-000.parquet` 的样本
- 产出到 `deploy/`：
  - `model.onnx`
  - `input_image.bin`
  - `input_state.bin`（已按 stats 做状态归一化）
  - `stats.json`
  - `golden.json`
  - `act_export_manifest.json`

说明：导出 wrapper 使用 `infer_cvae=false` 固定 latent 分支，以保证 ONNX 推理路径确定性。

### 3) 主机态推理复测（真实 output，通过）

执行：

```bash
cargo run --release --manifest-path apps/starry/act-infer-qemu/act-infer/Cargo.toml -- \
  /workspace/tgoskits/www/proj57/AKA-Sim2Real/deploy
```

输出：

```text
ACT_ACTION=[0.0016485304, -0.0040805936, 0.0]
```

### 4) QEMU riscv64 端到端复测（真实 output，通过）

执行：

```bash
env -u LD_PRELOAD ACT4STARRY_ROOT=/workspace/tgoskits/www/proj57/AKA-Sim2Real \
  cargo xtask starry app run -t act-infer-qemu --arch riscv64
```

关键串口输出：

```text
ACT_INFER_BEGIN
ACT_ACTION=[0.0016485378, -0.0040806173, 0.0]
ACT_INFER_OK
=== SUCCESS PATTERN MATCHED: (?m)^ACT_INFER_OK\s*$ ===
```

## 关键排障（已修复）

此前遇到过 riscv64 根盘识别失败：

```text
failed to determine root device from available block devices
```

根因是 app 的 riscv64 build features 未包含 virtio/pci 驱动。已在
`apps/starry/act-infer-qemu/build-riscv64gc-unknown-none-elf.toml` 补齐：

- `ax-hal/riscv64-qemu-virt`
- `ax-driver/pci`
- `ax-driver/virtio-blk`
- `ax-driver/virtio-net`
- `ax-driver/virtio-gpu`
- `ax-driver/virtio-input`
- `ax-driver/virtio-socket`

## 当前结论

- `apps/starry/act-infer-qemu` 的 app 组织、prebuild、overlay 注入、host/riscv 双构建链路可用。
- 已改用 `www/proj57/AKA-Sim2Real/output` 的真实训练产物（模型 + 数据）生成部署资产。
- 主机态推理与 QEMU riscv64 端到端均通过，成功命中 `ACT_INFER_OK`。
