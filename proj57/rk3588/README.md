# proj57/rk3588 - ACT 模型 RK3588 NPU 推理

本目录提供 RK3588 上运行 ACT 模型推理所需的说明和程序入口。这里默认使用**已经包含 StarryOS、rootfs 和 overlay 的现成镜像**；如果需要了解镜像如何合成，见 `image-build/README.md`。

镜像下载：https://pan.baidu.com/s/1VXZX3rQ0YlFrzpttvK6VoA?pwd=qfu9

## 适用范围

- 板端环境：Orange Pi 5 Plus / RK3588 / StarryOS
- 推理后端：RKNPU2
- 运行方式：启动进入 StarryOS 后，执行推理命令

## 镜像与启动

1. 将现成镜像烧录到 TF 卡。
2. 插卡上电，进入 StarryOS。
3. 登录串口 shell 后，直接运行推理程序。

如果需要镜像制作过程，参考 `image-build/README.md`；如果需要完整的板端复现背景，参考 `docs/share-rk3588-act-starry.md`。

## 推理命令

进入 StarryOS 后，直接执行以下命令进行左转的单步推理：

```sh
/act_infer_rk3588/run-review.sh left
```

常用变体：

```sh
/act_infer_rk3588/run-review.sh right
/act_infer_rk3588/run-golden.sh
```

`run-review.sh` 用于方向判断，`run-golden.sh` 用于与基准结果对照。

## 程序如何工作

推理程序会按以下顺序执行：

1. 读取输入图片和状态文件。
2. 完成图像缩放、归一化和状态归一化。
3. 通过 `librknnrt.so` 调用 RK3588 NPU。
4. 取回模型输出并做动作反归一化。
5. 根据左右轮速度差输出方向判断。

方向规则为：`right_wheel - left_wheel > 0` 判定为 `left`，小于 0 判定为 `right`。

## 输出内容

程序会打印结构化结果，核心字段包括：

- `backend`
- `left_wheel` / `right_wheel`
- `speed_diff`
- `direction`
- `output_action_norm`
- `output_action_denorm`
- `timing_ms`
- `peak_rss_kb`

其中：

- `direction` 是做差获得的方向判断。
- `timing_ms` 是分阶段耗时统计。
- `peak_rss_kb` 是进程峰值内存，StarryOS 对其支持较差。

`run-golden.sh` 还会输出 `ACT_INFER_OK`，表示基准对照通过。

## 依赖与目录

板端运行目录为 `/act_infer_rk3588/`，其中应包含：

- `run-review.sh`
- `run-golden.sh`
- `act-infer-review-rknn`
- `act-infer-golden-rknn`
- `lib/`
- `model/`

运行时依赖由镜像中的 overlay 提供，包含 `librknnrt.so` 和 glibc 相关加载库。

## 相关文档

- 资产准备和镜像合成：`image-build/README.md`
- 板上启动与推理复现：`docs/share-rk3588-act-starry.md`
- 详细实现说明：`docs/act-infer-report.md`
