# Proj57 RK3588 阶段交付报告

## 1. 交付目标

本阶段交付内容对应赛题任务二：在 Orange Pi 5 Plus / RK3588 开发板上启动 StarryOS，并通过 RK3588 NPU 完成 ACT 模型推理。交付结果覆盖 PyTorch 到 ONNX/RKNN 的模型转换、StarryOS 板端部署、用户态 RKNPU2 推理、动作后处理、golden 对照和左右转 review 样例验证。

## 2. 交付范围

| 项目 | 内容 |
| --- | --- |
| 平台 | Orange Pi 5 Plus，RK3588 |
| 操作系统 | StarryOS |
| 推理后端 | RK3588 NPU，RKNPU2 runtime |
| 语言 | Rust，FFI 调用 RKNPU2 C API |
| 模型格式 | ONNX 转 RKNN，目标平台 `rk3588` |
| 输入 | RGB 图像、机器人状态 `[left_vel, right_vel]` |
| 输出 | 8 步动作 chunk，每步 `[left_vel, right_vel, gripper_target]` |

## 3. 目录交付物

| 路径 | 说明 |
| --- | --- |
| `proj57/rk3588/act-infer/` | Rust 板端推理程序，包含 golden / review 两类入口。 |
| `proj57/rk3588/assets/prepare/` | ONNX、RKNN、统计量、golden 和左右转样例资产。 |
| `proj57/rk3588/assets/sdk/` | RKNPU2 runtime 运行依赖和头文件。 |
| `proj57/rk3588/scripts/` | 模型准备、交叉编译、部署和运行脚本。 |
| `proj57/rk3588/image-build/` | StarryOS ACT 镜像和 rootfs overlay 构建脚本。 |
| `proj57/rk3588/docs/reproduce.md` | 板端复现流程。 |
| `proj57/rk3588/docs/log/starry-act.log` | StarryOS 板端实测输出。 |
| `proj57/rk3588/docs/log/*.json` | Linux 对照和分项 JSON 记录。 |

## 4. 实现说明

### 4.1 模型转换

ACT checkpoint 先通过 `assets/prepare/export_onnx.py` 导出为 ONNX。导出阶段固定 checkpoint 中的 `inference_latent_mu`，运行时输入保持为图像和状态。ONNX 模型随后通过 `assets/prepare/convert_rknn.py` 转换为 RKNN，目标平台为 `rk3588`，转换策略采用 FP16 非量化，以减少方向判断中的数值偏差。

### 4.2 预处理

板端程序按赛题规格执行输入预处理：

1. 图像从 JPEG 解码为 RGB。
2. 图像 resize 到 `224x224`。
3. 图像按 `mean=[0.485, 0.456, 0.406]`、`std=[0.229, 0.224, 0.225]` 做归一化。
4. 状态按 `2 * (x - q01) / (q99 - q01) - 1` 做 QUANTILES 归一化。

### 4.3 NPU 推理与后处理

板端程序在用户态加载 `librknnrt.so`，通过 RKNPU2 runtime 完成输入 tensor 设置、`rknn_run`、输出 tensor 获取和释放。模型输出为归一化动作块，程序按 `(action + 1) / 2 * (q99 - q01) + q01` 反归一化。review 用例使用第一个时间步的左右轮速度判断方向：`right_wheel - left_wheel > 0` 为左转，`right_wheel - left_wheel < 0` 为右转。

## 5. 构建和复现

### 5.1 模型资产准备

在 `proj57/rk3588` 下执行：

```bash
bash scripts/prepare-model.sh
```

该步骤生成 `assets/prepare/model.onnx`、`assets/prepare/model.rknn`、`assets/prepare/stats.json`、`assets/prepare/golden.json`、`assets/prepare/input.jpg`、`assets/prepare/review_left.jpg` 和 `assets/prepare/review_right.jpg`。

### 5.2 板端程序构建

在 `proj57/rk3588` 下执行：

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install -y gcc-aarch64-linux-gnu
bash scripts/build-rk3588.sh
```

该步骤生成 `install/rk3588_linux_aarch64/act_infer_rk3588/`，其中包含板端二进制、`librknnrt.so`、glibc 动态加载器、模型资产和运行脚本。

### 5.3 StarryOS 镜像生成

在 tgoskits 工作区准备 rootfs 和镜像：

```bash
cargo xtask starry rootfs --arch aarch64
cd proj57/rk3588/image-build
./build-overlay-rootfs.sh starry-rootfs-act-infer.ext4
./make-dtb.sh
./repack-fit.sh
ROOTFS=starry-rootfs-act-infer.ext4 ./build-image.sh output/rk3588-starryos-act-infer.img
xz -T0 -6 -k output/rk3588-starryos-act-infer.img
```

### 5.4 板端运行

开发板从 TF 卡启动进入 StarryOS 后，执行：

```sh
/act_infer_rk3588/run-golden.sh
/act_infer_rk3588/run-review.sh left
/act_infer_rk3588/run-review.sh right
```

复现成功标志包括 `ACT_INFER_OK`、`ACT_REVIEW_DIRECTION=left`、`ACT_REVIEW_DIRECTION=right` 和 `ACT_REVIEW_DONE`。

## 6. 产物大小

| 产物 | 字节数 | 约合大小 |
| --- | ---: | ---: |
| `assets/prepare/model.onnx` | 202,634,593 | 193.25 MiB |
| `assets/prepare/model.rknn` | 101,498,896 | 96.80 MiB |
| `act-infer-golden-rknn` aarch64 release | 856,688 | 836.61 KiB |
| `act-infer-review-rknn` aarch64 release | 856,688 | 836.61 KiB |

## 7. StarryOS 板端运行时资源

以下数据来自 `proj57/rk3588/docs/log/starry-act.log`，测试平台为 Orange Pi 5 Plus / RK3588 / StarryOS。

| 用例 | NPU 核心策略 | `run_count` | `infer_single_ms` | `infer_total_ms` | `peak_rss_kb` |
| --- | --- | ---: | ---: | ---: | ---: |
| golden | auto | 20 | 37.67 | 753.37 | 25800 |
| golden | all | 20 | 29.37 | 587.47 | 25800 |
| review left | auto | 20 | 37.62 | 752.45 | 25760 |
| review left | all | 20 | 29.35 | 587.06 | 25760 |
| review right | auto | 20 | 37.64 | 752.72 | 25760 |
| review right | all | 20 | 29.38 | 587.65 | 25760 |

`ACT_CORE_MASK=all` 下的单次端到端推理耗时稳定在约 29.35 至 29.38 ms。StarryOS 输出中的 `peak_rss_kb` 为进程峰值内存采样，当前样例约 25.16 至 25.20 MiB。

## 8. 推理结果

### 8.1 golden 对照

StarryOS 板端 golden 用例输出如下：

| 指标 | `ACT_CORE_MASK=auto` | `ACT_CORE_MASK=all` |
| --- | ---: | ---: |
| `passed` | `true` | `true` |
| `max_abs_diff` | 0.0002 | 0.0002 |
| `infer_single_ms` | 37.67 | 29.37 |
| `peak_rss_kb` | 25800 | 25800 |

golden 首步反归一化动作为：

| 维度 | 数值 |
| --- | ---: |
| `left_vel` | -0.0015 |
| `right_vel` | 0.007 |
| `gripper_target` | 0.0 |

### 8.2 左转 review 样例

StarryOS 板端左转样例输出如下：

| 指标 | `ACT_CORE_MASK=auto` | `ACT_CORE_MASK=all` |
| --- | ---: | ---: |
| `left_wheel` | -0.0015 | -0.0015 |
| `right_wheel` | 0.007 | 0.007 |
| `speed_diff` | 0.0089 | 0.0089 |
| `direction` | left | left |
| `infer_single_ms` | 37.62 | 29.35 |
| `peak_rss_kb` | 25760 | 25760 |

该样例中 `right_wheel > left_wheel`，符合左转判定。

### 8.3 右转 review 样例

StarryOS 板端右转样例输出如下：

| 指标 | `ACT_CORE_MASK=auto` | `ACT_CORE_MASK=all` |
| --- | ---: | ---: |
| `left_wheel` | 0.006 | 0.006 |
| `right_wheel` | -0.00039 | -0.00039 |
| `speed_diff` | -0.0066 | -0.0066 |
| `direction` | right | right |
| `infer_single_ms` | 37.64 | 29.38 |
| `peak_rss_kb` | 25760 | 25760 |

该样例中 `right_wheel < left_wheel`，符合右转判定。

## 9. 结论

RK3588 阶段已完成 ACT 模型在 StarryOS 板端的 NPU 推理链路。golden 用例通过基准对照，左右转 review 样例均输出正确方向；`ACT_CORE_MASK=all` 下单次端到端推理约 29.35 ms，进程峰值内存约 25.2 MiB。交付内容覆盖赛题要求的模型转换、平台适配、StarryOS 部署、推理结果、产物大小、运行时内存占用和推理耗时记录。

## 10. AI 使用说明

RK3588 阶段未借鉴其他参赛队伍或个人的代码。镜像结构设计参考了 Armbian 镜像结构和 Orange Pi 官方镜像结构，启动链采用了 Armbian 较新版本的启动链组织方式，并结合 StarryOS 的内核、rootfs 和 overlay 需求完成适配。

**人工部分：**

一半以上的文档撰写、早期镜像结构研究和搭建探索、早期开发工作流尝试、开发容器到开发板的调试路径探索、所有镜像相关资源的准备工作、NPU 运行验证、所有板上测试工作、RK3588 的网口和串口调试、RK3588 uboot 调试、镜像构建和烧录、StarryOS 启动问题排查、推理程序 debug 和 glibc-musl 环境支持、推理程序输出的记录和验收、推理程序分段计时的升级等。

**AI 协作部分：**

主要用于将 QEMU 阶段推理程序迁移到 RK3588 阶段并适配 RKNPU2 runtime，迁移部分 QEMU 模型资产准备流程，参与约一半的文档润色和格式对齐，辅助分析镜像文档和启动链材料，固化镜像构建工作流，并整理相关脚本和复现流程。AI 的工作均经过人工 review。
