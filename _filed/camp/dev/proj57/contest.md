赛题背景

本赛题旨在推动国产操作系统StarryOS与前沿具身智能技术ACT模型的深度融合。通过在国产化平台上部署与优化模仿学习模型，验证系统对复杂AI任务的支持能力，促进自主可控的智能生态建设，培养跨学科人才，加速具身智能从研究走向产业落地。
赛题任务

使用提供的统一 ACT 模型，在 StarryOS 上完成 ACT 推理，输出预期数据。

    任务一：在荔枝派 sg2002（内存 256Mb）板子上跑通 ACT 推理
    任务二：在香橙派 rk3588（内存 4G）上跑通 ACT 推理
    任务三：在 QEMU 中跑通 ACT 模型推理

赛题特征

    软硬件协同，要求选手同时具备操作系统、驱动、AI推理部署等多方面能力。

    推理性能与资源约束并存，嵌入式平台资源受限（内存、算力），要求选手进行：模型裁剪 / 量化（可选）、推理框架适配（如 ONNX Runtime）、性能优化（启动时间、推理延迟）。

参考资料

    StarryOS：https://github.com/Starry-OS/StarryOS
    荔枝派相关：https://wiki.sipeed.com/hardware/zh/lichee/RV_Nano/5_peripheral.html
    ACT论文：Learning Fine-Grained Bimanual Manipulation with Low-Cost Hardware
    RK3588 模型转换工具：https://github.com/airockchip/rknn-toolkit2

评审要点

    任务一：在荔枝派 sg2002（256Mb）板子上跑通 ACT 推理。
    任务二：在香橙派 rk3588（4G）上跑通 ACT 推理。
    任务三：在 qemu 中跑通 ACT 模型推理。

该赛题的完成难度大致可以分为以下几个层次：

    基础难度： 完成任务三（在 QEMU 环境中基于c/c++/ rust实现cpu上运行 ACT 模型推理）
    进阶难度： 完成任务三 + 任务二（在 RK3588 平台基于c/c++/rust实现npu推理运行）
    优秀难度： 完成任务一（在 SG2002 256MB 平台基于c/c++/rust实现tpu推理运行）
    卓越难度： 同时完成任务一和任务二，并具备良好的工程实现与优化表现

需要特别说明的是，上述只是对题目本身完成难度的分析，并非最终的评奖标准。最终的奖项评定将由技术委员会评审专家根据大家的实际完成情况、技术方案、工程实现和优化表现等综合评定
