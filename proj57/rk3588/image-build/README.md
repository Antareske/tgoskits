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
| `starry-image.fit` | 由 `starryos.bin` + `starry.dtb` 打包的 FIT |

换新内核：替换 `starryos.bin`；换新 rootfs/overlay：替换 `starry-rootfs.ext4`。

## 脚本

所有脚本在本目录内执行，默认读写本目录下的同名资产。

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
- 从零合成：`./make-dtb.sh` 生成 starry.dtb，把新 `starryos.bin` /
  `starry-rootfs.ext4` 放进本目录 → `./repack-fit.sh` → `./build-image.sh`。

压缩与烧录：

```bash
xz -T0 -6 -k ../rk3588-starryos-smp8.img
sudo dd if=../rk3588-starryos-smp8.img of=/dev/sdX bs=4M conv=fsync && sync
```

## 依赖工具

`gdisk(sgdisk) mtools(mformat/mcopy/mdir) u-boot-tools(mkimage/dumpimage) dtc python3 xz dd`
