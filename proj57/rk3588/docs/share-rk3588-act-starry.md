# RK3588 + StarryOS 跑 ACT NPU 推理步骤说明

目标：在 Orange Pi 5 Plus 上，把 ACT 模型跑在 **StarryOS 纯板（无 Linux）+ RK3588 NPU**，输出左/右转决策。

---

## Step 1 — 准备模型资产（PC）

```bash
cd proj57/rk3588
bash scripts/prepare-model.sh
```

产出 `assets/prepare/`：
- `model.rknn`（FP16，≈97 MB）
- `stats.json`（归一化统计量）
- `golden.json`（基准输出）
- 样例图 `input.jpg`、`review_left.jpg`、`review_right.jpg`

**说明**：

- FP16 非量化。ACT 含 CVAE+Transformer，INT8 易翻转符号；板内存充足，优先决策方向正确性。

> 注意：rknn-toolkit2 2.4.2 要求 `onnx ≤ 1.17`，不能用 1.18+。

---

## Step 2 — 交叉编译推理程序（PC）

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install -y gcc-aarch64-linux-gnu

bash scripts/build-rk3588.sh
```

产出 `install/rk3588_linux_aarch64/act_infer_rk3588/`：
- `act-infer-review-rknn`（左/右决策二进制，≈824 KB）
- `lib/librknnrt.so` + glibc 运行时（`libc.so.6`、`libstdc++.so.6`、`ld-linux-aarch64.so.1` 等）
- `model/`（model.rknn + 样例图）
- `run-review.sh`（启动器，自动设 `LD_LIBRARY_PATH`）

**决断**：目标三元组用 `aarch64-unknown-linux-gnu`（不用 musl）。`librknnrt.so` 依赖 glibc 符号，必须走 gnu。rootfs 是 musl/Alpine，但 glibc 运行时一并打进 `lib/`，靠 `$ORIGIN/lib` rpath 自洽，不污染系统。

---

## Step 3 — 编译 StarryOS 内核 bin（PC，tgoskits 工作区）

```bash
# 确认 max_cpu_num=8
grep max_cpu_num os/StarryOS/configs/board/orangepi-5-plus.toml

cargo xtask starry quick-start orangepi-5-plus build
```

产出：`target/aarch64-unknown-linux-musl/release/starryos.bin`

**注意**：SMP 核数编译期固化，改了 `max_cpu_num` 后须删缓存再构建：
```bash
rm -f tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
```

---

## Step 4 — 构建 overlay rootfs（带 ACT 程序的 ext4）

先取 StarryOS 基础 rootfs：
```bash
cargo xtask starry rootfs --arch aarch64
# 产出: /tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img
```

再把 Step 2 的 `act_infer_rk3588/` 整目录写入 ext4 根目录 `/act_infer_rk3588`，得到 `starry-rootfs-act-infer.ext4`（可用 loop mount 或 debugfs 写入）。

写入后验证：
```bash
debugfs -R "ls -l /act_infer_rk3588" starry-rootfs-act-infer.ext4
```

---

## Step 5 — 合成整盘镜像

> **关键**：必须用 tgoskits 官方 DTB（`os/StarryOS/configs/board/orangepi-5-plus.dtb`），NPU 节点为 `rockchip,rk3588-rknpu`，与 StarryOS RKNPU 驱动匹配。Armbian 的 DTB 用的是 `rockchip,rk3588-rknn-core`，NPU 无法 probe。

```bash
cd proj57/rk3588/image-build

./make-dtb.sh               # 官方 DTB → starry.dtb（改写 bootargs，删 initrd）
./repack-fit.sh              # starryos.bin + starry.dtb → starry-image.fit
ROOTFS=starry-rootfs-act-infer.ext4 ./build-image.sh output/rk3588-starryos-act-infer.img
```

产出 `proj57/rk3588/image-build/output/rk3588-starryos-act-infer.img`，分区布局：
```
sector 64     idbloader.img   (DDR init + SPL)
sector 16384  u-boot.itb      (U-Boot + ATF/TEE)
p1 FAT        boot.scr + starry-image.fit + starry.dtb
p2 ext4       starry-rootfs-act-infer  (含 /act_infer_rk3588)
```

压缩：
```bash
xz -T0 -6 -k output/rk3588-starryos-act-infer.img
```

烧录 TF 卡后插板上电，串口 1500000 baud，自动引导到 StarryOS shell。

---

## Step 6 — 板上测试

串口接入（`picocom -b 1500000 /dev/ttyUSB0`），启动成功后：

```sh
root@starry:/root # /act_infer_rk3588/run-review.sh left
```

期望输出：
```
ACT_REVIEW_CASE=left
ACT_INFER_BEGIN
{ "backend": "rknn-npu", "timing_ms": { "infer_single_ms": 1123, "model_load_ms": 893 }, "peak_rss_kb": 24928, ... }
ACT_REVIEW_DIRECTION=left
ACT_REVIEW_DONE
```

测 right 用例（用全部 NPU 核）：
```sh
ACT_CORE_MASK=all /act_infer_rk3588/run-review.sh right
# 末尾: ACT_REVIEW_DIRECTION=right
```

输出 JSON 包含：`infer_single_ms`（单次推理耗时）、`model_load_ms`（模型加载耗时）、`peak_rss_kb`（进程峰值内存，来自 `/proc/self/status` VmHWM）。
