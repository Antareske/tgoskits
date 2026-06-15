# RK3588 交付说明

本文档汇总 `proj57/rk3588` 的赛题交付内容，覆盖模型转换、用户态推理程序、StarryOS 镜像制作、板端部署与验证结果。目录内的核心交付面向 Orange Pi 5 Plus（RK3588）上的 StarryOS + RKNPU2 用户态推理链路，目标是完成 ACT 模型的板上推理、方向判断与结果复现。

## 1. 交付范围

交付内容分为四部分：

1. PC 侧模型准备与转换。
2. RK3588 用户态推理程序与运行时封装。
3. StarryOS 整盘镜像与 ACT overlay 生成。
4. 板端复现步骤与验证材料。

## 2. 目录中的交付物

| 路径 | 说明 |
| --- | --- |
| `act-infer/` | Rust 用户态推理程序，包含 golden / review 两个入口。 |
| `assets/prepare/` | 模型导出、RKNN 转换、golden 生成所需的脚本与中间资产。 |
| `assets/sdk/` | `librknnrt.so` 与配套头文件。 |
| `scripts/` | 模型准备、交叉编译、部署到板、板上启动脚本。 |
| `image-build/` | StarryOS 整盘镜像与 ACT overlay 的合成工具。 |
| `docs/` | 交付说明、镜像构建说明、调试工作流与板上复现文档。 |
| `docs/log/` | 板测过程记录与结果 JSON。 |

## 3. 方案摘要

### 3.1 模型链路

ACT 模型先从 PyTorch checkpoint 导出为 ONNX，再转换为 RKNN。导出阶段固定 CVAE latent，使输入图与状态输入保持确定性；转换阶段采用 `rk3588` 目标平台与 **FP16 非量化** 策略，以降低数值偏差并保持左/右转方向稳定。

### 3.2 运行时链路

板端程序使用 Rust 编写，通过 FFI 直接调用 RKNPU2 runtime 的 C API。程序在用户态完成图像预处理、状态归一化、NPU 推理、动作反归一化与方向判断，最终输出结构化 JSON 与 `ACT_REVIEW_DIRECTION=<left|right|straight>` 形式的结果。

### 3.3 系统约束

1. 交叉编译目标采用 `aarch64-unknown-linux-gnu`。
2. `librknnrt.so` 所需的 glibc 动态加载器与共享库一并打包进安装目录。
3. StarryOS 镜像使用官方 `orangepi-5-plus.dtb` 作为基底，NPU 节点必须是 `rockchip,rk3588-rknpu`。
4. 板端 rootfs 采用 ext4 overlay 方式承载 `/act_infer_rk3588`。

## 4. 产物清单

### 4.1 模型与资产

- `assets/prepare/model.rknn`：FP16 RKNN 模型。
- `assets/prepare/stats.json`：归一化统计量。
- `assets/prepare/golden.json`：golden 基准输出。
- `assets/prepare/input.jpg`、`review_left.jpg`、`review_right.jpg`：验证样例。

### 4.2 可执行文件与运行时

- `install/rk3588_linux_aarch64/act_infer_rk3588/act-infer-golden-rknn`
- `install/rk3588_linux_aarch64/act_infer_rk3588/act-infer-review-rknn`
- `install/rk3588_linux_aarch64/act_infer_rk3588/lib/librknnrt.so`
- `install/rk3588_linux_aarch64/act_infer_rk3588/lib/ld-linux-aarch64.so.1`
- `install/rk3588_linux_aarch64/act_infer_rk3588/lib/libc.so.6` 等 glibc 运行时库

### 4.3 板端运行脚本

- `run-golden.sh`
- `run-review.sh`

## 5. 验证结果

### 5.1 功能验证

- `golden` 用例用于比对 NPU 输出与预生成基准。
- `review` 用例用于输出方向判断并支撑比赛复现。
- 左转样例与右转样例均已通过，方向判定与样例语义一致。

### 5.2 性能记录

程序内已内置分阶段计时与峰值内存采样字段，包含：

- `model_load_ms`
- `preprocess_ms`
- `normalize_state_ms`
- `inputs_set`
- `run`
- `outputs_get`
- `outputs_release`
- `npu_run`
- `denormalize_ms`
- `infer_single_ms`
- `infer_total_ms`
- `peak_rss_kb`

其中 `timing_ms` 与 `peak_rss_kb` 需在真实板端实测记录，结果会写入输出 JSON。

## 6. 交付建议

1. 先完成 `scripts/prepare-model.sh`，生成模型与 golden 资产。
2. 再执行 `scripts/build-rk3588.sh`，生成板端可部署目录。
3. 使用 `image-build/build-overlay-rootfs.sh` 与 `image-build/build-image.sh` 生成 ACT 镜像。
4. 最后按 `docs/share-rk3588-act-starry.md` 进行板上复现与验收。

## 7. 结论

当前交付覆盖 ACT 模型在 RK3588 + StarryOS 环境中的用户态 NPU 推理、板端镜像集成、部署与复现路径，满足从 PC 准备到板上验证的完整链路。
