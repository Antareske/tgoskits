# RK3588 纯 StarryOS 小镜像构建方案

## 目标

本文评估并拟定 Orange Pi 5 Plus / RK3588 的纯 StarryOS TF/SD 卡镜像方案。目标是做出类似 `www/tgoskits/doc/sg2002-starryos-image-guide.md` 的小镜像：

- 不保留 Armbian/Linux 维护 rootfs。
- 从 Armbian 原始镜像提取 RK3588 板级启动资产。
- 使用 tgoskits 构建 StarryOS FIT 和 DTB。
- 使用 tgoskits managed `rootfs-aarch64-alpine.img` 作为 `starry-rootfs`。
- `starry-rootfs` 原始大小保持 1GiB，减少烧录时间。
- FAT boot 分区保存 U-Boot 通过 `fatload` 加载的 StarryOS 资产。

该方案面向“快速烧录、直接启动 StarryOS”的板测卡，不保留可回退进入 Armbian Linux 的能力。

## 依据

### Rockchip Partitions 原文要点

`www/wiki-Partitions.txt` 给出的默认 storage map：

```text
GPT:      LBA 0 ~ LBA 63
loader1:  sector 64      size 7104 sectors   preloader / idbloader
loader2:  sector 16384   size 8192 sectors   U-Boot or UEFI
trust:    sector 24576   size 8192 sectors   ATF / OP-TEE, miniloader path only
boot:     sector 32768   size 229376 sectors kernel, dtb, extlinux.conf, ramdisk
rootfs:   sector 262144  size remaining       Linux system
```

同文档 Note 1 指出：

```text
SPL with trust support -> loader2 is available for u-boot.itb
trust partition not available / not used
```

这与 Orange Pi 5 Plus 指南中“U-Boot 2025.04 / SPL + u-boot.itb”路径一致。

### Rockchip Boot Option 原文要点

`www/wiki-boot-option.txt` 给出的 boot stage：

```text
stage 2: idbloader.img @ 0x40
stage 3: u-boot.itb    @ 0x4000, with SPL path and including U-Boot + ATF
stage 4: boot.img      @ 0x8000
stage 5: rootfs.img    @ 0x40000
```

同文档对 SD/TF 卡启动给出写入方式：

```bash
dd if=idbloader.img of=sdb seek=64
dd if=u-boot.itb    of=sdb seek=16384
dd if=boot.img      of=sdb seek=32768
dd if=rootfs.img    of=sdb seek=262144
```

还说明 `boot.img` 是可包含 kernel Image 和 DTB 的已知文件系统镜像，格式可以是 FAT 或 EXT2；也可以理解为 boot 分区里的 boot folder / boot assets。

### Orange Pi 指南要点

`www/OrangePi5Plus_StarryOS_指南.md` 中与本方案直接相关的结论：

- `0x40`、`0x4000`、`0x8000`、`0x40000` 都是 512B LBA sector 编号，不是字节偏移。
- U-Boot 2025.04 走 SPL + `u-boot.itb`，不需要单独 `trust.img`。
- Stage 4 boot 区域可以保存 FAT/vfat `/boot` 内容。
- StarryOS 资产推荐放在 boot 分区：`starry-image.fit` 和 `starry.dtb`。
- U-Boot 启动 StarryOS 的核心命令是 `fatload` FIT/DTB 后 `bootm`。
- Orange Pi 5 Plus 的 U-Boot 网络路径不应默认依赖，优先走 SD FAT `fatload`。

## Armbian 原始镜像静态分析

输入镜像：

```text
www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz
```

解压后的原始布局：

```text
Disk: 3219456 sectors, about 1.54 GiB
Partition table: GPT
p1: start 32768, end 3217407, ext4, name rootfs
```

注意：这份 Armbian minimal 镜像不是独立 FAT boot + ext4 rootfs 双分区模型。它只有一个 ext4 `rootfs` 分区，Armbian 的 `/boot` 文件位于 ext4 内部。

但是 `p1` 前面的 Rockchip 固定区域可提取启动资产：

```text
sector 64:    magic 534e4b52, Rockchip boot image / idbloader area
sector 16384: magic edfe0dd0, FIT / DTB magic, u-boot.itb area
```

字符串检查确认包含：

```text
U-Boot SPL 2026.01_armbian-...
U-Boot 2026.01_armbian-...
FIT image for U-Boot with bl31 (TF-A)
Rockchip Boot Image
```

Armbian ext4 `/boot` 中还可提取：

```text
/boot/boot.scr
/boot/boot.cmd
/boot/armbianEnv.txt
/boot/dtb/rockchip/rk3588-orangepi-5-plus.dtb
```

其中 `armbianEnv.txt` 指定：

```text
fdtfile=rockchip/rk3588-orangepi-5-plus.dtb
```

## 可行性结论

方案可行。理由：

- Rockchip 原文明确支持 SD/TF 卡从固定扇区加载 `idbloader.img`、`u-boot.itb`、boot assets 和 rootfs。
- Armbian 原始镜像中已经包含可复用的 RK3588 `idbloader` 和 `u-boot.itb` 区域。
- Orange Pi 指南已验证 U-Boot 可以从 boot 分区通过 `fatload` 加载 `starry-image.fit` 和 `starry.dtb`，再 `bootm`。
- tgoskits 已提供 aarch64 Alpine managed rootfs，原始大小 1GiB，适合直接作为小镜像的 rootfs 内容。

主要限制：

- 纯 StarryOS 镜像不保留 Armbian Linux 维护入口。
- Armbian minimal 镜像没有独立 FAT boot 分区，不能直接“改名复用”；需要重新拼 GPT 和 FAT boot 分区。
- 从 Armbian 原始镜像提取的 `idbloader` / `u-boot.itb` 是二进制资产，需保留来源记录并用真机验证。
- 如果板子只从 SPI Flash 启动 U-Boot，SD 上的 loader 区域可能不会参与启动；但为了做完整可移植 SD 镜像，仍建议写入 loader 区域。
- StarryOS 当前 root 选择不支持 `root=UUID=...`，应使用 `root=PARTLABEL=starry-rootfs`。

## 推荐镜像布局

使用 GPT，保留 Rockchip 默认关键 sector：

```text
RK3588 pure StarryOS image, about 1.2 GiB

raw area:
├── LBA 0-63:        protective MBR + primary GPT
├── LBA 64:          idbloader.img / loader1 area
├── LBA 16384:       u-boot.itb / loader2 area
├── LBA 24576:       trust area, unused for SPL + u-boot.itb path
├── p1 boot:         start 32768, FAT32, 112 MiB
│   ├── starry-image.fit
│   ├── starry.dtb
│   └── 可选 boot.scr
└── p2 starry-rootfs: start 262144, ext4, 1 GiB
    └── rootfs-aarch64-alpine.img 内容
```

分区建议：

```text
p1: start 32768,  end 262143,  size 112 MiB, FAT32, name boot
p2: start 262144, size 1 GiB, ext4, name/PARTLABEL starry-rootfs
```

镜像总大小建议：

```text
rootfs end + GPT backup + padding
约 1.2 GiB
```

如果需要给 StarryOS rootfs 留更多运行空间，可以把 p2 设为 2GiB 或 4GiB；若目标是类似 SG2002 的最小小镜像，则保持 1GiB。

## FAT Boot 分区职责

FAT boot 分区用于 U-Boot Stage 4 资产加载。U-Boot 从该分区读取 StarryOS FIT 和 DTB：

```text
fatload mmc ${dev}:1 ${loadaddr} starry-image.fit
fatload mmc ${dev}:1 ${fdt_addr_r} starry.dtb
setenv bootargs root=PARTLABEL=starry-rootfs
bootm ${loadaddr} - ${fdt_addr_r}
```

如果 `starry-image.fit` 已内嵌正确 DTB，理论上可以省略单独 `starry.dtb`；但 Orange Pi 指南明确要求 `bootm ${loadaddr} - ${fdt_addr_r}`，且旧 U-Boot 只执行 `bootm ${loadaddr}` 可能解析失败。因此标准镜像仍建议放置外置 `starry.dtb`。

可选 `boot.scr`：

- 如果希望上电自动进入 StarryOS，可在 FAT 中放置 `boot.scr`，内容执行上述 `fatload` 和 `bootm`。
- 如果只做手工验证，可以不放 `boot.scr`，进入 U-Boot 后手动输入命令。
- 如果 U-Boot 默认 bootcmd 不扫描该 FAT 分区或不执行 `boot.scr`，自动启动仍需调试 U-Boot 环境；手动 `fatload` 路径不受影响。

## 资产来源

### 从 Armbian 原始镜像提取

假设原始 raw 镜像为：

```text
www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_original.img
```

提取 loader 区域：

```bash
mkdir -p target/rk3588-pure-starry/assets

dd if=www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_original.img \
  of=target/rk3588-pure-starry/assets/idbloader.img \
  bs=512 skip=64 count=8128 status=none

dd if=www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_original.img \
  of=target/rk3588-pure-starry/assets/u-boot.itb \
  bs=512 skip=16384 count=8192 status=none
```

说明：

- `idbloader.img` 按 loader1 最大区域提取，可能包含尾部零填充。
- `u-boot.itb` 按 loader2 4MiB 区域提取，可能包含尾部零填充。
- 对于 `dd` 写回固定扇区，带尾部零填充的区域镜像是可接受的。
- 若后续要把文件作为独立发布资产，可进一步按 FIT/loader 实际长度裁剪，但不是拼 raw SD 镜像的必要条件。

提取 Armbian DTB 作为参考或备用：

```bash
mkdir -p /tmp/armbian-rootfs

mount -o ro,loop,offset=$((32768*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_original.img \
  /tmp/armbian-rootfs

cp /tmp/armbian-rootfs/boot/dtb/rockchip/rk3588-orangepi-5-plus.dtb \
  target/rk3588-pure-starry/assets/armbian-rk3588-orangepi-5-plus.dtb

cp /tmp/armbian-rootfs/boot/boot.scr \
  target/rk3588-pure-starry/assets/armbian-boot.scr

cp /tmp/armbian-rootfs/boot/boot.cmd \
  target/rk3588-pure-starry/assets/armbian-boot.cmd

cp /tmp/armbian-rootfs/boot/armbianEnv.txt \
  target/rk3588-pure-starry/assets/armbianEnv.txt

umount /tmp/armbian-rootfs
```

标准 Starry 镜像应优先使用 tgoskits 的 board DTB：

```text
os/StarryOS/configs/board/orangepi-5-plus.dtb
```

Armbian DTB 可作为对照和调试备用。

### 从 tgoskits 构建 StarryOS FIT/DTB

构建：

```bash
cargo xtask starry quick-start orangepi-5-plus build
```

根据 Orange Pi 指南，生成产物需复制为：

```text
starry-image.fit
starry.dtb
```

当前指南中的典型来源：

```text
<quick-start workdir>/image.fit -> starry-image.fit
os/StarryOS/configs/board/orangepi-5-plus.dtb -> starry.dtb
```

真机路径应保持：

```text
max_cpu_num = 1
```

避免 RK3588 SMP 触发已知 `task stack guard page TLB shootdown timeout`。

### 下载 StarryOS rootfs

```bash
cargo xtask starry rootfs --arch aarch64
```

或手工下载：

```bash
mkdir -p tmp/axbuild/rootfs

curl -L \
  -o tmp/axbuild/rootfs/rootfs-aarch64-alpine.img.tar.xz \
  https://github.com/rcore-os/tgosimages/releases/download/v0.0.5/rootfs-aarch64-alpine.img.tar.xz

tar -C tmp/axbuild/rootfs \
  -xf tmp/axbuild/rootfs/rootfs-aarch64-alpine.img.tar.xz
```

得到：

```text
tmp/axbuild/rootfs/rootfs-aarch64-alpine.img
```

该 rootfs 原始大小为 1GiB。

## 构建流程草案

### 1. 创建 raw 镜像

示例创建约 1.2GiB 镜像。实际大小只需覆盖 p2 末尾和 secondary GPT。

```bash
IMG=target/rk3588-pure-starry/rk3588-pure-starry.img
mkdir -p target/rk3588-pure-starry
truncate -s 1280M "$IMG"
```

### 2. 创建 GPT 分区

```bash
sgdisk --clear "$IMG"

sgdisk \
  -n 1:32768:262143 \
  -t 1:0700 \
  -c 1:boot \
  "$IMG"

sgdisk \
  -n 2:262144:+1G \
  -t 2:8300 \
  -c 2:starry-rootfs \
  "$IMG"

sgdisk -v "$IMG"
```

说明：

- p1 起点 `32768` 对应 Rockchip boot stage `0x8000`。
- p2 起点 `262144` 对应 Rockchip rootfs stage `0x40000`。
- p1 使用 GPT type `0700` 是为了 FAT/data 分区兼容；也可根据项目习惯改为 Linux filesystem type。
- p2 的 GPT name 即 `PARTLABEL=starry-rootfs`。

### 3. 写入 Rockchip loader 区域

```bash
dd if=target/rk3588-pure-starry/assets/idbloader.img \
  of="$IMG" bs=512 seek=64 conv=notrunc status=none

dd if=target/rk3588-pure-starry/assets/u-boot.itb \
  of="$IMG" bs=512 seek=16384 conv=notrunc status=none
```

不要向 `0x6000` 写 `trust.img`，因为 SPL + `u-boot.itb` 路径下 ATF/BL31 已包含在 `u-boot.itb` 中。

### 4. 格式化并填充 FAT boot 分区

安装工具：

```bash
apt-get install -y dosfstools mtools
```

格式化 FAT：

```bash
mkfs.vfat -F 32 -n BOOT --offset=32768 "$IMG" 229376
```

如果 `mkfs.vfat --offset` 不可用，可改用 loop offset 方式格式化。

复制文件可用 mtools 或 loop mount。mtools 示例：

```bash
cat > target/rk3588-pure-starry/mtoolsrc <<EOF
drive b: file="$IMG" offset=$((32768*512))
EOF

MTOOLSRC=target/rk3588-pure-starry/mtoolsrc mcopy \
  target/rk3588-pure-starry/assets/starry-image.fit \
  b::starry-image.fit

MTOOLSRC=target/rk3588-pure-starry/mtoolsrc mcopy \
  target/rk3588-pure-starry/assets/starry.dtb \
  b::starry.dtb
```

可选生成 `boot.scr`，用于自动启动 StarryOS：

```bash
cat > target/rk3588-pure-starry/boot.cmd <<'EOF'
setenv loadaddr 0x10000000
setenv fdt_addr_r 0x0a100000
setenv bootargs root=PARTLABEL=starry-rootfs
fatload mmc ${devnum}:1 ${loadaddr} starry-image.fit || fatload mmc 0:1 ${loadaddr} starry-image.fit || fatload mmc 1:1 ${loadaddr} starry-image.fit
fatload mmc ${devnum}:1 ${fdt_addr_r} starry.dtb || fatload mmc 0:1 ${fdt_addr_r} starry.dtb || fatload mmc 1:1 ${fdt_addr_r} starry.dtb
bootm ${loadaddr} - ${fdt_addr_r}
EOF

mkimage -A arm64 -T script -C none \
  -n 'StarryOS RK3588 boot script' \
  -d target/rk3588-pure-starry/boot.cmd \
  target/rk3588-pure-starry/assets/boot.scr
```

是否放置 `boot.scr` 取决于是否要自动启动。初期建议先保留手动 U-Boot 命令路径，验证成功后再启用自动脚本。

### 5. 写入 1GiB starry-rootfs

```bash
P2_START=262144
P2_SIZE_SECTORS=$((1024*1024*1024/512))

P2_LOOP=$(losetup --find --show \
  --offset $((P2_START*512)) \
  --sizelimit $((P2_SIZE_SECTORS*512)) \
  "$IMG")

dd if=tmp/axbuild/rootfs/rootfs-aarch64-alpine.img \
  of="$P2_LOOP" bs=4M conv=fsync status=progress

tune2fs -L starry-rootfs "$P2_LOOP"
e2fsck -f -y "$P2_LOOP"
losetup -d "$P2_LOOP"
```

如果 p2 恰好为 1GiB，与 rootfs 镜像同尺寸，则不需要 `resize2fs`。如果 p2 大于 1GiB，则写入后应运行：

```bash
resize2fs "$P2_LOOP"
```

### 6. 验证

GPT：

```bash
sgdisk -v "$IMG"
sgdisk -p "$IMG"
parted -s "$IMG" unit s print free
```

loader magic：

```bash
od -An -tx4 -N4 -j $((64*512)) "$IMG"
od -An -tx4 -N4 -j $((16384*512)) "$IMG"
```

预期：

```text
sector 64:    534e4b52
sector 16384: edfe0dd0
```

FAT boot 内容：

```bash
mdir -i "$IMG"@@$((32768*512)) ::
```

rootfs：

```bash
P2_LOOP=$(losetup --find --show \
  --offset $((262144*512)) \
  --sizelimit $((1024*1024*1024)) \
  "$IMG")

e2fsck -f -n "$P2_LOOP"
tune2fs -l "$P2_LOOP" | grep -E 'Filesystem volume name|Block count|Block size|Free blocks'
losetup -d "$P2_LOOP"
```

### 7. 压缩

```bash
xz -T0 -6 -k -c "$IMG" > "$IMG.xz"
xz -t "$IMG.xz"
xz -l "$IMG.xz"
```

## 真机启动流程

### 手动启动

进入 U-Boot 后：

```text
setenv loadaddr 0x10000000
setenv fdt_addr_r 0x0a100000
setenv bootargs root=PARTLABEL=starry-rootfs
fatload mmc 0:1 ${loadaddr} starry-image.fit
fatload mmc 0:1 ${fdt_addr_r} starry.dtb
bootm ${loadaddr} - ${fdt_addr_r}
```

如果 SD 是 `mmc 1`，改为：

```text
fatload mmc 1:1 ${loadaddr} starry-image.fit
fatload mmc 1:1 ${fdt_addr_r} starry.dtb
bootm ${loadaddr} - ${fdt_addr_r}
```

### 自动启动

若 U-Boot 默认 bootcmd 会扫描 FAT boot 分区并执行 `boot.scr`，可将 StarryOS `boot.scr` 放入 p1，实现上电自动进入 StarryOS。

如果自动启动失败，优先回到手动命令验证：

```text
mmc list
mmc dev 0
fatls mmc 0:1
fatls mmc 1:1
```

确认 U-Boot 能看到 `starry-image.fit` 和 `starry.dtb`。

## 与保留 Armbian 方案的对比

| 方案 | 镜像大小 | 烧录速度 | Linux 维护入口 | Starry rootfs | 适用场景 |
|------|----------|----------|----------------|---------------|----------|
| 保留 Armbian + starry-rootfs | 13GiB/29GiB 等 | 较慢 | 有 | 1GiB/17GiB 可选 | 调试和救援方便 |
| 纯 StarryOS 小镜像 | 约 1.2GiB | 快 | 无 | 1GiB | 快速刷卡、专用板测 |

纯 StarryOS 小镜像更接近 SG2002 模型，适合稳定后作为标准板测卡基础镜像。早期 bring-up 仍建议保留一张 Armbian 维护卡，避免 U-Boot、DTB 或 rootfs 选择错误后必须重新烧卡救援。

## 风险和待验证项

- 从 Armbian 提取的 `idbloader.img` 和 `u-boot.itb` 是否在所有 Orange Pi 5 Plus 板卡修订上稳定启动，需要真机验证。
- 当前 Armbian U-Boot 版本字符串为 `2026.01_armbian`，而 Orange Pi 指南中多处以 `2025.04` 为已知可靠路径；二者启动 Starry FIT 的兼容性应通过真机 `bootm` 验证。
- FAT boot 分区类型、bootable attribute、U-Boot 自动扫描策略可能影响自动启动，但不影响手动 `fatload`。
- `boot.scr` 中 `${devnum}`、`mmc 0/1` 的选择依赖板上是否有 eMMC 或 SPI U-Boot 环境；自动脚本需要实测调优。
- StarryOS 使用 `root=PARTLABEL=starry-rootfs`，必须确认 GPT 分区名正确写入且 StarryOS root 选择代码能识别该块设备。
- p2 只有 1GiB，适合基础 rootfs 和小型测试；大型压力测试、数据库测试或在线上传大量资产时可能需要更大的 rootfs 或额外 testdata 分区。

## 建议落地顺序

1. 从 Armbian 原始镜像提取 loader 区域和 Orange Pi DTB，保存为可复用资产。
2. 用 tgoskits 构建 `starry-image.fit` 和 `starry.dtb`。
3. 下载 `rootfs-aarch64-alpine.img`。
4. 拼装 1.2GiB 纯 StarryOS raw 镜像。
5. 烧录到 TF 卡，先手动 U-Boot `fatload` + `bootm`。
6. 验证 StarryOS 挂载的是 `PARTLABEL=starry-rootfs`。
7. 手动启动稳定后，再增加 `boot.scr` 自动启动。
8. 最后将流程脚本化为 `cargo xtask starry board-image build orangepi-5-plus --pure` 或独立 shell 脚本。
