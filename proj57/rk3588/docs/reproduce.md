# RK3588 基础要求复现文档：开机 Starry 并跑通 ACT 推理

本文档给出 Orange Pi 5 Plus（RK3588）上电启动 StarryOS、进入串口终端并完成 ACT 推理验证的最短复现路径。复现目标包含三项：StarryOS 正常启动、`review` 用例跑通、`golden` 用例跑通。

## 1. 资产准备

复现前需要先准备三类资产，且其中两类直接依赖 `tgoskits`：

- 模型资产（proj57 赛题仓库）
- 板端运行资产（用户态 ACT 推理程序及其相关资产）
- 整盘镜像资产（由 `tgoskits` 生成）。

`tgoskits` 在这条链路里负责三件事：编译 StarryOS 内核、下载基础 rootfs、合成整盘镜像。`proj57/rk3588` 则负责 ACT 模型、推理程序和板端部署目录，两边合起来才构成完整复现材料。

### 1.1 模型资产

在 `proj57/rk3588` 目录下执行：

```bash
bash scripts/prepare-model.sh
```

该步骤生成以下文件：

- `assets/prepare/model.rknn`
- `assets/prepare/stats.json`
- `assets/prepare/golden.json`
- `assets/prepare/input.jpg`
- `assets/prepare/review_left.jpg`
- `assets/prepare/review_right.jpg`

这些文件分别用于 NPU 推理、归一化、golden 校验和左右转样例验证。

### 1.2 板端运行资产

在 `proj57/rk3588` 目录下执行：

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install -y gcc-aarch64-linux-gnu
bash scripts/build-rk3588.sh
```

该步骤生成 `install/rk3588_linux_aarch64/act_infer_rk3588/`，目录内应包含：

- `act-infer-review-rknn`
- `act-infer-golden-rknn`
- `lib/librknnrt.so`
- `lib/ld-linux-aarch64.so.1`
- `lib/libc.so.6` 等 glibc 运行时库
- `model/model.rknn`
- `model/stats.json`
- `model/golden.json`
- `model/input.jpg`
- `model/input_state.bin`
- `model/review_left.jpg`
- `model/review_right.jpg`
- `run-review.sh`
- `run-golden.sh`

### 1.3 整盘镜像资产

先准备 `tgoskits` 项目，在 `tgoskits` 中使用脚本生成 overlay 与整盘镜像。

> 编译 StarryOS 前，先阅读 **1.4** 确认配置信息。

```bash
cargo xtask starry rootfs --arch aarch64
cd proj57/rk3588/image-build
./build-overlay-rootfs.sh starry-rootfs-act-infer.ext4
./make-dtb.sh
./repack-fit.sh
ROOTFS=starry-rootfs-act-infer.ext4 ./build-image.sh output/rk3588-starryos-act-infer.img
xz -T0 -6 -k output/rk3588-starryos-act-infer.img
```

该步骤生成可直接烧录的 `output/rk3588-starryos-act-infer.img.xz`。

有关镜像构建的详细说明，见 `docs/develop/rk3588-starryos-build-manual.md`。

### 1.4 StarryOS 编译前配置

`tgoskits` 中 `os/StarryOS/configs/board/orangepi-5-plus.toml` 是复现前必须核对的编译配置。该配置决定 Orange Pi 5 Plus 的 StarryOS 内核如何构建，关键项如下：

```toml
target = "aarch64-unknown-none-softfloat"
log = "Warn"
max_cpu_num = 8
plat_dyn = true
```

- `max_cpu_num = 8`：SMP 核数按 8 核编译，运行时不再变更。
- `log = "Warn"`：编译和运行日志保持在警告级别，可选 `Info` 和 `Warn`。宜选 `Warn` 避免 `Info` 频繁输出调试信息，增加 ACT 推理用时。
- `plat_dyn = true`：启用动态平台配置，配合该板型的 quick-start 流程。

构建前还应确认缓存模板与源配置一致，且构建产物全新；若模板仍保留旧值或 target 含旧产物，需要删除后重新生成：

```bash
grep max_cpu_num tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
rm -f tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
rm -rf target/aarch64-unknown-none-softfloat/release/starryos*
```

### 1.5 复现前检查

1. 已准备好 `proj57/rk3588/image-build/output/rk3588-starryos-act-infer.img.xz`，或等价的 ACT 镜像。
2. 已准备 TF 卡与读卡器。
3. 已准备 CH340 TTL 串口线，波特率固定为 `1500000`。
4. 开发板已刷入镜像并可从 TF 卡启动。

## 2. 烧录镜像

镜像可直接使用压缩包烧录，或先解压后烧录。解压方式如下：

```bash
xz -dk proj57/rk3588/image-build/output/rk3588-starryos-act-infer.img.xz
```

随后使用 balenaEtcher 或等价工具将镜像写入 TF 卡。

## 3. 启动 StarryOS

1. 将 TF 卡插入开发板。
2. 接好串口线并打开串口终端，如使用 `MobaXterm`，串口波特率设为 `1500000`。
3. 上电启动开发板。

Linux 可使用 `picocom` 登录串口：

```bash
picocom -b 1500000 /dev/ttyUSB0
```

启动成功后，串口会出现 StarryOS 启动信息，并最终进入 shell：

```text
Welcome to Starry OS!
root@starry:/root #
```

## 4. 确认 ACT 目录

进入 shell 后，先确认部署目录存在：

```sh
ls /act_infer_rk3588
```

目录内应包含：

- `run-review.sh`
- `run-golden.sh`
- `act-infer-review-rknn`
- `act-infer-golden-rknn`
- `lib/`
- `model/`

## 5. 跑通 review 用例

先验证左转样例：

```sh
/act_infer_rk3588/run-review.sh left
```

期望结尾至少包含：

```text
ACT_REVIEW_CASE=left
ACT_REVIEW_DIRECTION=left
ACT_REVIEW_DONE
```

再验证右转样例：

```sh
ACT_CORE_MASK=all /act_infer_rk3588/run-review.sh right
```

期望结尾至少包含：

```text
ACT_REVIEW_CASE=right
ACT_REVIEW_DIRECTION=right
ACT_REVIEW_DONE
```

`ACT_REPEAT` 可用于重复推理采样，例如：

```sh
ACT_REPEAT=20 ACT_CORE_MASK=auto /act_infer_rk3588/run-review.sh left
```

## 6. 跑通 golden 用例

golden 用例用于对比 RKNN 输出与基准值：

```sh
/act_infer_rk3588/run-golden.sh
```

期望结尾至少包含：

```text
ACT_INFER_OK
```

## 7. 输出结果说明

review / golden 两类命令都会输出结构化 JSON，核心字段包括：

- `backend`
- `left_wheel` / `right_wheel`
- `speed_diff`
- `direction`
- `output_action_norm`
- `output_action_denorm`
- `timing_ms`
- `peak_rss_kb`

其中 `timing_ms` 包含模型加载、预处理、输入设置、NPU 执行、输出获取和后处理等分段耗时；`peak_rss_kb` 为进程峰值内存。

## 8. 常见问题

1. 若命令报 `not found`，通常是 glibc 加载器或运行时库未随镜像正确部署。
2. 若 `rknpu` 相关调用报 NotFound，通常是设备树使用了错误的 DTB，需确认采用官方 `orangepi-5-plus.dtb`，可从 `tgoskits` 中获取。
3. 若串口无输出，优先检查 TTL 线序、波特率与串口设备名。
4. 若 `run-review.sh` 找不到样例图或模型文件，检查 `/act_infer_rk3588/model/` 是否完整，串口输入的命令是否有截断。

## 9. 复现完成判定

以下结果同时满足时，可判定复现完成：

- StarryOS 可从 TF 卡正常启动并进入 shell。
- `run-review.sh left` 输出 `ACT_REVIEW_DIRECTION=left`。
- `run-review.sh right` 输出 `ACT_REVIEW_DIRECTION=right`。
- `run-golden.sh` 输出 `ACT_INFER_OK`。
