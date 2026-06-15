# RK3588 StarryOS 镜像资产与换新工具

本目录把「合成整盘镜像」所需的各类原材料单独抽离出来，并提供脚本，方便后续频繁换新
StarryOS 内核或 rootfs(+overlay)，无需每次从 Armbian 提取启动链、从 tgoskits 取设备树。

完整原理见 `../docs/rk3588-starryos-build-manual.md`。

## 资产清单

稳定资产（一般不动）：

| 文件 | 说明 | 来源 | 镜像中的位置 |
| --- | --- | --- | --- |
| `idbloader.img` | DDR init + SPL | Armbian 镜像 | sector 64 |
| `u-boot.itb` | U-Boot + ATF/TEE (FIT) | Armbian 镜像 | sector 16384 |
| `starry.dtb` | 已写好 `bootargs` 的设备树 | tgoskits 官方 DTB | FAT boot 分区 p1 + 打进 FIT |
| `boot.scr` | U-Boot 自动引导脚本 | 本目录 `boot.cmd` 编译 | FAT boot 分区 p1 |
| `boot.cmd` | `boot.scr` 的源文 | — | （仅源文，不入盘） |
| `starry.its` | FIT 打包模板 | — | （仅模板，不入盘） |

> `starry.dtb` 取自 tgoskits 官方 `os/StarryOS/configs/board/orangepi-5-plus.dtb`
> （NPU 节点 `rockchip,rk3588-rknpu`，与 StarryOS 驱动匹配），只改写 `/chosen/bootargs`。
> **不要**用 Armbian 镜像里的 DTB——其 NPU 节点为 `rockchip,rk3588-rknn-core`，StarryOS
> 驱动不识别，NPU 无法初始化。详见构建手册 2.3 / 4.1。

频繁换新的资产：

| 文件 | 说明 |
| --- | --- |
| `starryos.bin` | StarryOS 内核裸二进制（编译期固化 SMP=8） |
| `starry-rootfs.ext4` | StarryOS 根文件系统（ext4，含用户态/overlay），1 GiB |
| `starry-rootfs-act-infer.ext4` | 在基础 rootfs 上叠加 ACT 推理 overlay 的 ext4（含 `/act_infer_rk3588` 与 glibc 运行时） |
| `starry-image.fit` | 由 `starryos.bin` + `starry.dtb` 打包的 FIT |

换新内核：替换 `starryos.bin`；换新 rootfs/overlay：替换 `starry-rootfs.ext4`（ACT 推理镜像则替换 `starry-rootfs-act-infer.ext4`）。

## ACT 推理 overlay + rootfs 构建

为 ACT 推理程序合成 rootfs 时，**仅把 `install/.../act_infer_rk3588/` 拷进根目录是不够的**。
ACT 二进制以 `aarch64-unknown-linux-gnu`（glibc）目标交叉编译，因为 `librknnrt.so`
依赖 glibc 符号；而 StarryOS 基础 rootfs 是 Alpine/musl，只自带 musl 运行时。
若 overlay 缺少 glibc 环境，程序会在 `exec` 阶段直接失败（找不到解释器或符号），
板测表现为 `not found` 或动态链接错误，而非推理逻辑问题。构建 overlay 时必须确保
以下环境与资产齐全。

### 必须齐全的运行时环境

1. **glibc 动态加载器（最易漏）**：ACT 二进制的 ELF 解释器是写死的绝对路径
   `/lib/ld-linux-aarch64.so.1`（见 `readelf -l <bin> | grep interpreter`）。
   内核 `exec` 时按此**绝对路径**寻找加载器，`$ORIGIN/lib` rpath 和
   `LD_LIBRARY_PATH` 都改不了它。Alpine 基础 rootfs 没有这个文件，必须把 glibc
   加载器装到 rootfs 的 `/lib/ld-linux-aarch64.so.1`（可用符号链接指向真正的
   `ld-linux-aarch64.so.1`，例如交叉工具链 sysroot 的
   `/usr/aarch64-linux-gnu/lib/ld-linux-aarch64.so.1`）。
2. **glibc 共享库**：`libc.so.6`、`libm.so.6`、`libgcc_s.so.1`、`libpthread.so.0`、
   `libdl.so.2`、`libstdc++.so.6`（→ `libstdc++.so.6.0.x`）等。这些与
   `librknnrt.so` 一起放在 `/act_infer_rk3588/lib`，由二进制的 `$ORIGIN/lib`
   rpath 解析；`run-*.sh` 还会显式 `export LD_LIBRARY_PATH=<here>/lib`，二者自洽，
   不污染系统 musl。注意 glibc 版本要与编译时一致（如 `libstdc++.so.6.0.35`），
   版本不匹配会出现符号缺失。
3. **NPU 内核侧前提（非 overlay 内容，但同属环境齐全）**：NPU 能否 probe 取决于
   设备树而非 rootfs。务必使用官方 DTB（`rockchip,rk3588-rknpu`），否则即便
   overlay 完整，运行时所有 rknpu ioctl 仍返回 NotFound。详见构建手册 2.3 / 4.1。

### 必须齐全的程序资产

`/act_infer_rk3588/` 下需具备（来自 `scripts/build-rk3588.sh` 打包的 install 目录）：

```
act-infer-golden-rknn / act-infer-review-rknn   可执行二进制
lib/librknnrt.so + 上述 glibc 运行时             运行时库
model/model.rknn                                FP16 RKNN 模型（≈97 MB）
model/stats.json                                QUANTILE 归一化统计量（不可缺，否则归一化错误）
model/golden.json                               golden 基准（golden 用例必需）
model/input.jpg                                 默认/golden 输入图
model/input_state.bin                           [left_vel,right_vel] 原始状态
model/review_left.jpg / review_right.jpg        review 左/右用例图
run-golden.sh / run-review.sh                   板上启动器（自动设 LD_LIBRARY_PATH）
```

缺任一项的影响：缺 `model.rknn`/`stats.json` 会在加载或归一化阶段失败；缺
`golden.json` 只影响 golden 用例；缺对应用例图会让该 case 报
`case image not found`。

### 构建步骤

`build-overlay-rootfs.sh` 把上述所有环境与资产需求固化成一个可复现的工作流，
无需手动处理 glibc / 加载器细节，执行完毕后自动校验：

```bash
# 1) 准备模型资产（只需一次，或模型更新时重跑）
cd ../  # proj57/rk3588
bash scripts/prepare-model.sh

# 2) 交叉编译并打包 install 目录（含二进制 + 模型 + 完整 glibc 运行时）
bash scripts/build-rk3588.sh

# 3) 取 StarryOS 基础 rootfs（必须用干净的 musl 基础，不能用上次的 overlay 产物）
cargo xtask starry rootfs --arch aarch64

# 4) 一键构建功能齐全的 overlay ext4（fakeroot + mke2fs -d，无需 root，fsck 干净）
cd image-build
./build-overlay-rootfs.sh starry-rootfs-act-infer.ext4
# 脚本会自动探测步骤 3 的产物路径，叠加 install 目录，
# 把 glibc 加载器接到 /lib/ld-linux-aarch64.so.1，并逐项校验通过后才退出。

# 5) 合成整盘镜像
./make-dtb.sh && ./repack-fit.sh
ROOTFS=starry-rootfs-act-infer.ext4 ./build-image.sh output/rk3588-starryos-act-infer.img
```

如需指定路径：
```bash
BASE_ROOTFS=/path/to/clean-base.ext4 ./build-overlay-rootfs.sh out.ext4
```
> 完整 PC→板端步骤见 `../docs/share-rk3588-act-starry.md`；ACT 程序与资产的来源、
> 交叉编译决断（为何用 gnu 而非 musl）见 `../README.md` 与
> `../docs/act-infer-report.md`。

## 脚本

所有脚本在本目录内执行，默认读写本目录下的同名资产。

### build-overlay-rootfs.sh — 构建 ACT 推理 overlay ext4

把 ACT 推理 install 目录叠加到 StarryOS 基础 musl rootfs，产出 fsck 干净的
`starry-rootfs-act-infer.ext4`。内部使用 `fakeroot` + `mke2fs -d`，无需 root，
自动补齐 glibc 运行时与加载器，完成后逐项校验。

```bash
./build-overlay-rootfs.sh [输出ext4]   # 默认 starry-rootfs-act-infer.ext4
# 环境变量: BASE_ROOTFS= INSTALL_DIR= DEST= MARGIN_MB=
```

**前提**：先跑过 `../scripts/build-rk3588.sh`（打包 install + glibc）和
`cargo xtask starry rootfs --arch aarch64`（取干净基础 rootfs）。

### repack-fit.sh

### make-dtb.sh — 从官方 DTB 生成 starry.dtb

从 tgoskits 官方 `os/StarryOS/configs/board/orangepi-5-plus.dtb` 生成带 StarryOS
bootargs 的 `starry.dtb`：保留/补齐 `stdout-path`，替换 `bootargs` 为
`root=PARTLABEL=starry-rootfs ...`，删除 `linux,initrd-*`。官方 DTB 的 NPU 节点为
`rockchip,rk3588-rknpu`，是 NPU 能在板上工作的前提。

```bash
./make-dtb.sh [官方dtb路径]   # 默认取工作区 os/StarryOS/configs/board/orangepi-5-plus.dtb
# 生成后如需写入 FIT/镜像，再跑 repack-fit.sh / swap-kernel.sh
```

### repack-fit.sh

用当前 `starryos.bin` + `starry.dtb` 重新打包 `starry-image.fit`。换内核或换设备树后调用。

```bash
./repack-fit.sh
```

### build-image.sh — 全量合成整盘镜像

把全部资产组装成一张可直接烧录的镜像。镜像大小按 rootfs 自动计算。

```bash
./build-image.sh [输出镜像路径]      # 默认输出 ../rk3588-starryos-smp8.img
# 可选: ROOTFS= FIT= DTB= SIZE_MB= 覆盖
```

### swap-kernel.sh — 只换内核/设备树（对已有镜像动刀）

不重建分区表与 rootfs，只重打 FIT 并写回 FAT boot 分区。

```bash
./swap-kernel.sh <镜像> [新内核bin] [新dtb]
# 例: 用 tgoskits 新编内核换进现成镜像
./swap-kernel.sh ../rk3588-starryos-smp8.img \
  ../../../target/aarch64-unknown-none-softfloat/release/starryos.bin
```

### swap-rootfs.sh — 只换 rootfs(+overlay)（对已有镜像动刀）

不动启动链/FAT boot/内核，只写 p2。新 rootfs 容量超出时自动增大镜像并重建 p2。

```bash
./swap-rootfs.sh <镜像> [新rootfs.ext4]
# 例: 用 tgoskits 下载的 rootfs
./swap-rootfs.sh ../rk3588-starryos-smp8.img \
  /tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img
```

## 典型工作流

- 只改内核：在工作区 `cargo xtask starry quick-start orangepi-5-plus build` →
  `./swap-kernel.sh ../rk3588-starryos-smp8.img <新>/starryos.bin` → 压缩烧录。
- 只换 rootfs：`cargo xtask starry rootfs --arch aarch64` →
  `./swap-rootfs.sh ../rk3588-starryos-smp8.img <下载的rootfs>` → 压缩烧录。
- 换设备树：`./make-dtb.sh`（官方 DTB 更新后重生成 starry.dtb）→
  `./repack-fit.sh` → `./swap-kernel.sh ../镜像` → 压缩烧录。
- **ACT 推理镜像（含 overlay）**：`../scripts/build-rk3588.sh` →
  `cargo xtask starry rootfs --arch aarch64` →
  `./build-overlay-rootfs.sh starry-rootfs-act-infer.ext4` →
  `./make-dtb.sh && ./repack-fit.sh` →
  `ROOTFS=starry-rootfs-act-infer.ext4 ./build-image.sh output/rk3588-starryos-act-infer.img`
  → 压缩烧录。
- 从零合成：`./make-dtb.sh` 生成 starry.dtb，把新 `starryos.bin` /
  `starry-rootfs.ext4` 放进本目录 → `./repack-fit.sh` → `./build-image.sh`。

压缩与烧录：

```bash
xz -T0 -6 -k ../rk3588-starryos-smp8.img
sudo dd if=../rk3588-starryos-smp8.img of=/dev/sdX bs=4M conv=fsync && sync
```

## 依赖工具

`gdisk(sgdisk) mtools(mformat/mcopy/mdir) u-boot-tools(mkimage/dumpimage) dtc python3 xz dd`

`build-overlay-rootfs.sh` 还需要：`fakeroot e2fsprogs(mke2fs/e2fsck/debugfs)`
