# RK3588 StarryOS ACT NPU 推理程序报告

## 简要介绍

本推理程序位于 `proj57/rk3588`，用于在 Orange Pi 5 Plus（RK3588）上通过 StarryOS 运行 ACT 模型的 RKNN/NPU 推理链路。程序在 PC 侧将 PyTorch/ONNX 模型转换为 RKNN 格式，在板端通过 Rust 用户态程序直接调用 RKNPU2 runtime（`librknnrt.so`）完成推理。

板端执行流程为：读取图片和状态输入，完成图像 resize、RGB 转换、ImageNet 归一化和状态归一化，随后通过 `rknn_inputs_set`、`rknn_run`、`rknn_outputs_get` 调用 NPU 推理，最后对动作输出反归一化并判断左右转方向。

程序提供两类入口：

- `golden`：将 NPU 输出与预先生成的 golden 输出做误差比对，用于验证 RKNN 推理结果正确性。
- `review`：输出左右轮速度、方向判断和 JSON 结果，用于比赛检查与人工复核。

## 结论

当前 StarryOS + RK3588 NPU 推理链路已经跑通。

| 项目 | 结果 |
| --- | --- |
| golden 用例 | 成功 |
| review 用例 | 成功 |
| 单次推理，`core-mask=auto` | 约 55 ms，通常为单 NPU 核 |
| 单次推理，三核 `core-mask=012/all` | 约 45 ms |

从结果看，三核模式相比 auto 单核模式有可见加速，说明 RKNN runtime 与 StarryOS RKNPU 驱动之间的多核提交链路有效。但该加速幅度不是 3 倍，原因包括模型图依赖、可并行部分比例、runtime 任务切分方式、输入输出搬运和同步开销等。

## 数据链路观察

用户态程序的预处理不是流式处理。图片通过 `image::open` 整体读取并解码到内存，再转换为 RGB、resize 到 224x224，最后遍历生成 NCHW float32 输入缓冲。状态文件和统计文件也采用一次性读取。

NPU 调用路径在用户态层面不是端到端零拷贝。程序先在 CPU 内存中构造 `Vec<f32>` 输入，再传给 `rknn_inputs_set`；输出侧通过 `rknn_outputs_get` 获取 runtime 缓冲后，还会复制到 Rust `Vec<f32>` 中再释放 runtime 输出。StarryOS 的 RKNPU 驱动内部使用 DMA buffer 与 cache sync，驱动到硬件的数据通路具备零拷贝语义，但当前 RKNN 用户态接口没有把应用侧输入输出直接绑定为驱动 DMA buffer。

因此，当前记录的单次推理耗时不是纯 NPU kernel 计算耗时，而是包含 `rknn_run` 前后 runtime/驱动输入输出准备、同步和输出获取等开销的端到端推理阶段耗时。

## 不足

- 用户态程序到驱动之间不是端到端零拷贝，输入输出存在额外 CPU 内存拷贝，开销较大。
- 当前推理时间包含 NPU 驱动输入、输出拷贝和同步开销，并非完全等价于纯 NPU 计算时长。
- 单次推理证明了 StarryOS + RK3588 NPU 链路可用，但实践意义有限，尚未覆盖连续输入、批量任务、长时间运行和真实控制闭环场景。
- 预处理不是流式处理，图片整读、整解码、整图 resize 和归一化都会占用额外内存与 CPU 时间。
- 模型加载涉及约 97 MB RKNN 文件的 I/O 和 runtime 初始化，加载时间较长，不适合频繁创建上下文的用法。
- 模型未做裁剪、蒸馏或 INT8 量化，FP16 RKNN 模型体积和计算量都偏大，整体方案较笨重。

## 后续优化方向

- 将模型上下文常驻，避免每次任务重复加载 RKNN 模型。
- 评估 RKNN runtime 是否支持用户预分配/透传 buffer，减少应用到 runtime 的输入输出拷贝。
- 将预处理改为更轻量的数据路径，例如直接读取目标尺寸输入、减少中间图像格式转换，或把部分预处理下沉到模型/硬件侧。
- 对模型做裁剪、量化或结构压缩，降低模型大小、加载时间和单次推理计算量。
- 增加连续推理压测，分别统计预处理、输入设置、NPU 执行、输出获取和后处理耗时，拆清瓶颈来源。
