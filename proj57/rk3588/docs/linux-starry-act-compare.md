# RK3588 ACT：Linux / StarryOS 性能对比

本次在同一块 OrangePi 5 Plus 开发板上，使用同一 ACT 单步推理程序，测得 Linux 与 StarryOS 的 ACT 性能对比，测试命令如下：

```bash
ACT_REPEAT=100 ACT_CORE_MASK=auto /act_infer_rk3588/run-golden.sh
ACT_REPEAT=100 ACT_CORE_MASK=all /act_infer_rk3588/run-golden.sh
ACT_REPEAT=100 ACT_CORE_MASK=auto /act_infer_rk3588/run-review.sh left
ACT_REPEAT=100 ACT_CORE_MASK=all /act_infer_rk3588/run-review.sh left
ACT_REPEAT=100 ACT_CORE_MASK=auto /act_infer_rk3588/run-review.sh right
ACT_REPEAT=100 ACT_CORE_MASK=all /act_infer_rk3588/run-review.sh right
```

使用来自 `proj57` 赛题的 ACT 推理模型，运行目标是 RK3588 上的 `act_infer_rk3588` 用例。

`ACT_CORE_MASK=auto` 表示让 runtime 自动选择 NPU 核心，`ACT_CORE_MASK=all` 表示使用全部可用 NPU 核心。

三种测试分别是 `golden`（和基准输出做一致性校验）、`review left`（左转场景）、`review right`（右转场景）的 ACT 单步推理，每种测试都分别覆盖 `ACT_CORE_MASK=auto` 和 `ACT_CORE_MASK=all`。

表格单位均为 `ms`，`Starry/Linux` 越大表示 StarryOS 该阶段越慢。`infer_total_ms` 受 `ACT_REPEAT` 影响，Linux 循环测试 100 次，Starry 为 20 次，因此不做直接横比。

## golden 用例

### `ACT_CORE_MASK=auto`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 122.46 | 778.88 | 6.36x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 63.35 | 88.68 | 1.40x | StarryOS 预处理略慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 4.21x | 绝对值很小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 4.57 | 12.35 | 2.70x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 19.41 | 22.18 | 1.14x | StarryOS NPU 执行略慢 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.05 | 0.14 | 2.61x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 10.60x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 19.48 | 22.52 | 1.16x | 与 `run` 基本一致 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 28.26 | 94.21 | 3.34x | StarryOS 首帧慢很多 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 10.00x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 24.08 | 37.67 | 1.56x | StarryOS 端到端更慢 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210856 | 25800 | -- | StarryOS 内存监控不完整 |

### `ACT_CORE_MASK=all`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 106.63 | 778.89 | 7.30x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 59.98 | 91.05 | 1.52x | StarryOS 预处理略慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 4.21x | 绝对值很小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 5.98 | 12.35 | 2.07x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 15.72 | 13.89 | 0.88x | StarryOS 此项略快 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.04 | 0.14 | 3.73x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 13.28x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 15.90 | 14.22 | 0.89x | 与 `run` 一致，StarryOS 略快 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 35.78 | 85.79 | 2.40x | StarryOS 首帧仍更慢 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 8.33x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 21.88 | 29.37 | 1.34x | StarryOS 端到端更慢 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210860 | 25800 | -- | StarryOS 内存监控不完整 |

## review left 用例

### `ACT_CORE_MASK=auto`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 126.14 | 778.94 | 6.17x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 7.08 | 87.67 | 12.39x | StarryOS 预处理显著更慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 24.14x | 绝对值极小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 4.34 | 12.35 | 2.85x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 18.05 | 22.18 | 1.23x | StarryOS NPU 执行更慢 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.05 | 0.14 | 2.74x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 10.82x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 18.05 | 22.48 | 1.24x | 与 `run` 基本一致 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 19.62 | 93.29 | 4.75x | StarryOS 首帧慢很多 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 8.67x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 22.42 | 37.62 | 1.68x | StarryOS 端到端更慢 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210800 | 25760 | -- | StarryOS 内存监控不完整 |

### `ACT_CORE_MASK=all`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 123.94 | 778.68 | 6.28x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 62.87 | 90.84 | 1.44x | StarryOS 预处理略慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 3.14x | 绝对值很小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 6.40 | 12.34 | 1.93x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 22.44 | 13.89 | 0.62x | StarryOS 此项更快 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.04 | 0.14 | 3.35x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 11.59x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 22.59 | 14.22 | 0.63x | 与 `run` 一致，StarryOS 略快 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 39.83 | 85.52 | 2.15x | StarryOS 首帧仍更慢 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 5.78x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 28.99 | 29.35 | 1.01x | 端到端几乎持平 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210788 | 25760 | -- | StarryOS 内存监控不完整 |

## review right 用例

### `ACT_CORE_MASK=auto`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 122.31 | 778.93 | 6.37x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 11.04 | 89.36 | 8.09x | StarryOS 预处理显著更慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 11.01x | 绝对值极小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 4.55 | 12.35 | 2.71x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 18.09 | 22.14 | 1.22x | StarryOS NPU 执行更慢 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.05 | 0.14 | 2.62x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 11.41x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 18.17 | 22.49 | 1.24x | 与 `run` 基本一致 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 28.25 | 94.26 | 3.34x | StarryOS 首帧慢很多 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 8.67x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 22.76 | 37.64 | 1.65x | StarryOS 端到端更慢 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210796 | 25760 | -- | StarryOS 内存监控不完整 |

### `ACT_CORE_MASK=all`

| 指标/阶段 | 阶段说明 | 涉及硬件/子系统 | Linux | StarryOS | Starry/Linux | 观察 |
|---|---|---|---:|---:|---:|---|
| `model_load_ms` | 模型加载与上下文初始化 | CPU / RKNN runtime | 122.40 | 780.07 | 6.37x | StarryOS 模型加载明显更慢 |
| `preprocess_ms` | 图片解码、缩放、归一化 | CPU | 13.94 | 92.32 | 6.62x | StarryOS 预处理明显更慢 |
| `normalize_state_ms` | 状态向量归一化 | CPU | 0.00 | 0.01 | 12.03x | 绝对值很小 |
| `inputs_set.avg_ms` | 输入 tensor 交给 RKNN runtime | CPU / RKNN runtime / DMA sync | 5.91 | 12.35 | 2.09x | StarryOS 输入设置更慢 |
| `run.avg_ms` | `rknn_run` wall time | RKNN runtime / NPU driver / NPU cores | 22.02 | 13.90 | 0.63x | StarryOS 此项更快 |
| `outputs_get.avg_ms` | 获取输出 tensor | RKNN runtime / 内存同步 / CPU copy | 0.04 | 0.14 | 3.75x | 输出获取更慢 |
| `outputs_release.avg_ms` | 释放输出 tensor | RKNN runtime | 0.00 | 0.00 | 11.37x | 绝对值极小 |
| `npu_run.avg_ms` | RKNN runtime 查询到的 NPU 真实执行时间 | NPU cores | 22.14 | 14.23 | 0.64x | 与 `run` 一致，StarryOS 略快 |
| `first_run_ms` | 首帧含 warmup 的耗时 | CPU + RKNN runtime + NPU | 35.08 | 85.81 | 2.45x | StarryOS 首帧仍更慢 |
| `denormalize_ms` | 动作反归一化 | CPU | 0.00 | 0.01 | 8.33x | 绝对值极小 |
| `infer_single_ms` | 单次端到端推理 | CPU + RKNN runtime + NPU + 内存同步 | 22.76 | 29.38 | 1.29x | StarryOS 端到端更慢 |
| `peak_rss_kb` | 进程峰值内存 | 进程内存 | 210780 | 25760 | -- | StarryOS 内存监控不完整 |

## 初步结论

1. StarryOS 的 `model_load_ms` 明显高于 Linux，说明模型上下文初始化成本偏大。
2. 端到端 `infer_single_ms` 在多数用例下 StarryOS 仍更慢，但 `ACT_CORE_MASK=all` 下的 `run.avg_ms` / `npu_run.avg_ms` 已接近甚至优于 Linux。
3. 预处理和 `outputs_get` 仍是 StarryOS 侧更明显的额外开销来源。
4. StarryOS 的内存监控支持不完善。
5. 实际测试时观察到 StarryOS 的 `infer_single_ms` 偏稳定，Linux 则浮动较大。
