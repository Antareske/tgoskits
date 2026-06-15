# Proj57 QEMU 阶段交付报告

## 1. 交付目标

本阶段交付内容对应赛题任务三：在 QEMU 环境中运行 StarryOS，并在 CPU 后端完成 ACT 模型推理。交付结果覆盖图像预处理、状态归一化、ONNX 模型推理、动作反归一化、golden 对照和左右转 review 样例验证。

## 2. 交付范围

| 项目 | 内容 |
| --- | --- |
| 平台 | QEMU riscv64，StarryOS guest |
| 推理后端 | CPU，`tract` ONNX 推理引擎 |
| 语言 | Rust |
| 模型格式 | PyTorch checkpoint 导出为 ONNX，推理时固定 CVAE latent |
| 输入 | RGB 图像、机器人状态 `[left_vel, right_vel]` |
| 输出 | 8 步动作 chunk，每步 `[left_vel, right_vel, gripper_target]` |

## 3. 目录交付物

| 路径 | 说明 |
| --- | --- |
| `proj57/qemu/act-infer/` | Rust 推理程序，包含 golden / review 两类入口。 |
| `proj57/qemu/assets/prepare/` | 模型导出、golden 生成、ONNX 一致性校验脚本和输入资产。 |
| `proj57/qemu/prebuild.sh` | StarryOS app 预构建脚本，负责编译 riscv64 用户态二进制并写入 overlay。 |
| `proj57/qemu/qemu-riscv64.toml` | golden 用例 QEMU 配置。 |
| `proj57/qemu/qemu-riscv64-review.toml` | review 用例 QEMU 配置。 |
| `proj57/qemu/act-infer-golden.sh` | StarryOS guest 内 golden 执行脚本。 |
| `proj57/qemu/act-infer-review.sh` | StarryOS guest 内 review 执行脚本。 |

## 4. 实现说明

### 4.1 模型导出

ACT checkpoint 通过 `assets/prepare/export_onnx.py` 导出为 ONNX。导出时将 checkpoint 中的 `inference_latent_mu` 固定进推理图，运行时输入保持为 `image` 和 `state`，避免在嵌入式侧引入随机 latent 采样。

### 4.2 预处理

推理程序按赛题规格执行输入预处理：

1. 图像从 JPEG 解码为 RGB。
2. 图像 resize 到 `224x224`。
3. 图像按 `mean=[0.485, 0.456, 0.406]`、`std=[0.229, 0.224, 0.225]` 做归一化。
4. 状态按 `2 * (x - q01) / (q99 - q01) - 1` 做 QUANTILES 归一化。

### 4.3 推理与后处理

StarryOS guest 内运行 `act-infer-*-tract` 二进制。模型输出为归一化动作块，程序按 `(action + 1) / 2 * (q99 - q01) + q01` 反归一化。review 用例使用第一个时间步的左右轮速度判断方向：`right_wheel - left_wheel > 0` 为左转，`right_wheel - left_wheel < 0` 为右转。

## 5. 构建和复现

### 5.1 环境依赖

QEMU 复现需要以下环境：

| 依赖 | 说明 |
| --- | --- |
| `cargo xtask` | tgoskits StarryOS app 构建入口。 |
| `riscv64gc-unknown-linux-musl` Rust target | 构建 StarryOS guest 用户态推理程序。 |
| `riscv64-linux-musl-gcc` | riscv64 musl 链接器。 |
| QEMU riscv64 | 启动 StarryOS guest。 |

### 5.2 golden 用例

在 tgoskits 工作区执行：

```bash
ACT_ASSETS_DIR=/abs/path/to/assets/prepare \
cargo xtask starry app qemu -t act-infer-qemu --arch riscv64
```

成功标志为：

```text
ACT_INFER_OK
```

### 5.3 review 用例

review 用例通过 `qemu-riscv64-review.toml` 启动，运行 `/usr/bin/act-infer-review.sh`。输入资产目录中 `input.jpg` 分别替换为 `review_left.jpg` 和 `review_right.jpg` 后执行：

```bash
ACT_ASSETS_DIR=/abs/path/to/runtime-assets \
cargo xtask starry app qemu \
  -t act-infer-qemu \
  --arch riscv64 \
  --qemu-config /abs/path/to/proj57/qemu/qemu-riscv64-review.toml
```

成功标志为：

```text
ACT_REVIEW_DONE
```

## 6. 产物大小

本次实测使用的 ONNX 资产来自同一套 ACT 导出链路。当前 `proj57/qemu/assets/prepare/` 未保留大模型文件，QEMU 现测使用 `proj57/rk3588/assets/prepare/model.onnx` 作为同源 ONNX 输入。

| 产物 | 字节数 | 约合大小 |
| --- | ---: | ---: |
| `model.onnx` | 202,634,593 | 193.25 MiB |
| `act-infer-golden-tract` riscv64 musl | 18,097,136 | 17.26 MiB |
| `act-infer-review-tract` riscv64 musl | 18,077,392 | 17.24 MiB |
| `act-infer-golden-tract` Linux host | 47,561,536 | 45.36 MiB |
| `act-infer-review-tract` Linux host | 47,543,808 | 45.34 MiB |

## 7. 运行时资源

| 项目 | 数值 | 说明 |
| --- | ---: | --- |
| QEMU guest 内存 | 1024 MiB | `qemu-riscv64.toml` 和 `qemu-riscv64-review.toml` 中 `-m 1024M`。 |
| StarryOS 启动后可用内存 | 1003.69 MiB | QEMU 启动日志中内存图显示的剩余 Free 区间。 |
| ONNX 模型文件 | 193.25 MiB | 运行时从 rootfs `/opt/act/model.onnx` 读取。 |
| 单个推理二进制 | 17.24 至 17.26 MiB | riscv64 musl release 产物。 |
| 进程峰值 RSS | 未单独输出 | QEMU 阶段推理程序当前输出 `timing_ms`，未内置 per-process RSS 字段；运行内存边界由 QEMU guest 1024 MiB 和模型/二进制文件大小记录。 |

## 8. 推理结果

### 8.1 golden 对照

QEMU/StarryOS 实测输出如下：

| 指标 | 数值 |
| --- | ---: |
| `passed` | `true` |
| `max_abs_diff` | 0.00011179224 |
| `run_count` | 1 |
| `infer_single_ms` | 25926.2326 |
| `infer_total_ms` | 25926.2326 |

golden 首步反归一化动作为：

| 维度 | 数值 |
| --- | ---: |
| `left_vel` | -0.0015871897 |
| `right_vel` | 0.007265431 |
| `gripper_target` | 0.0 |

### 8.2 左转 review 样例

| 指标 | 数值 |
| --- | ---: |
| `ACT_REVIEW_CASE` | `left` |
| `left_wheel` | -0.0015871897 |
| `right_wheel` | 0.007265431 |
| `speed_diff` | 0.008852621 |
| 判定方向 | left |
| `infer_single_ms` | 25232.9138 |

该样例中 `right_wheel > left_wheel`，符合左转判定。

### 8.3 右转 review 样例

| 指标 | 数值 |
| --- | ---: |
| `ACT_REVIEW_CASE` | `right` |
| `left_wheel` | 0.006104648 |
| `right_wheel` | -0.00044651033 |
| `speed_diff` | -0.0065511586 |
| 判定方向 | right |
| `infer_single_ms` | 25378.738999999998 |

该样例中 `right_wheel < left_wheel`，符合右转判定。

## 9. 结论

QEMU 阶段已完成 ACT 模型在 StarryOS guest 内的 CPU 推理链路。golden 用例通过基准对照，左转和右转 review 样例均输出正确方向，交付内容覆盖赛题要求的模型导出、平台适配、StarryOS 运行、推理结果、产物大小、运行时内存边界和推理耗时记录。

## 10. AI 使用说明

QEMU 阶段未借鉴其他参赛队伍或个人的代码。实现过程中参考了赛题提供的 ACT 模型规格、数据格式、预处理和后处理要求，并基于 StarryOS / tgoskits 的既有 app 运行机制完成集成。

人工完成的主要工作包括核心推理代码的大部分实现、项目架构组织、大部分文档内容撰写、基于 tgoskits 开发环境的 StarryOS QEMU 用例测试、golden / review 两类用例的部分实现和推理程序输出结果的验收。

AI 协作主要用于 CI 适配、模型导出脚本整理、推理代码的优化、资产准备流程辅助、部分脚本细节补全、文档的润色和对齐。AI 参与内容均经过人工 review。
