# SG2002 StarryOS 镜像构建与快速更新脚本

对应 `sg2002-starryos-image-build-guide.md` 的自动化脚本，支持完整镜像构建
和开发迭代中快速更新内核/rootfs（基于已有镜像创建新镜像，不修改原文件）。

所有脚本从仓库根目录执行。

## 资产清单

SG2002 StarryOS 镜像依赖以下资产，其中标注 **【项目资产】** 的必须使用项目自带版本，
不可从 Linux 镜像提取。

| 资产 | 来源 | 大小 | 说明 |
|------|------|------|------|
| **【项目资产】** `os/StarryOS/configs/board/licheerv-nano-sg2002.dtb` | 项目仓库 | ~24 KB | 设备树，必须使用项目版本（与 StarryOS 驱动匹配），Linux 镜像中的 DTB (~22 KB) 会导致启动失败 |
| **【项目资产】** StarryOS 内核 `starryos.bin` | `cargo xtask` 编译 | ~14 MB | 由脚本自动编译 |
| **【项目资产】** ITS 文件（内嵌于脚本） | 构建指南 | — | FIT image 描述文件，引导 U-Boot 加载 kernel + ramdisk + DTB |
| fip.bin | SG2002 Linux 镜像 | 440832 字节 | Cvitek 一级 bootloader (FSBL + BL31 + U-Boot SPL)，不可自行构建 |
| ramdisk.bin | SG2002 Linux 镜像 | 2527648 字节 | Cvitek 平台固化 ramdisk |
| rootfs | `cargo xtask starry rootfs --arch riscv64` | 2 GB | Alpine Linux rootfs (ext4) |

### fip.bin / ramdisk.bin 一次性提取

这两个文件只需从 SG2002 Linux 镜像（如 Sipeed 官方固件）提取一次，
之后存放在 `target/sg2002/` 下复用。

```bash
# 假设 Linux 镜像为 sg2002_linux.img，boot 分区起始扇区为 2048
BOOT_OFF=$((2048 * 512))
mcopy -i "sg2002_linux.img@@${BOOT_OFF}" ::fip.bin target/sg2002/fip.bin
mcopy -i "sg2002_linux.img@@${BOOT_OFF}" ::boot.sd /tmp/linux_boot.sd
dumpimage -T flat_dt -p 1 -o target/sg2002/ramdisk.bin /tmp/linux_boot.sd
```

## 脚本

### build_image.sh — 完整构建

从零开始编译内核并组装 SD 卡镜像。

```bash
./www/sg2002-image-scripts/build_image.sh
```

**预置条件**：`target/sg2002/fip.bin` 和 `target/sg2002/ramdisk.bin` 已就位。

**产出**：`target/sg2002/sg2002_starryos_wifi.img` (2.1 GB)。

**执行步骤**：编译内核 → 验证 fip.bin/ramdisk.bin → 复制项目 DTB → 生成 boot.sd → 下载 rootfs → 组装镜像。

### update_kernel.sh — 更新内核

编译新 StarryOS 内核并创建新 SD 卡镜像（boot.sd FIT image 为新内核）。
源镜像不受影响，产出为独立的新镜像文件。

```bash
./www/sg2002-image-scripts/update_kernel.sh [SRC_IMAGE] [OUT_IMAGE]
```

- `SRC_IMAGE`：源镜像（不会被修改），默认 `target/sg2002/sg2002_starryos_wifi.img`
- `OUT_IMAGE`：新镜像路径，默认在源镜像同目录生成 `*_kernel-YYYYMMDD-HHMMSS.img`

**说明**：`cp` 源镜像 → 编译新内核 → 重新生成 `boot.sd` → `mcopy -o` 写入新镜像的 boot 分区。
只更新 boot 分区中的 `boot.sd`，分区表、rootfs、fip.bin、ramdisk.bin 保持不变。

### update_rootfs.sh — 更新 rootfs

创建新 SD 卡镜像，其中 ext4 rootfs 分区被替换。源镜像不受影响。

```bash
./www/sg2002-image-scripts/update_rootfs.sh [SRC_IMAGE] [ROOTFS_IMG] [OUT_IMAGE]
```

- `SRC_IMAGE`：源镜像（不会被修改），默认 `target/sg2002/sg2002_starryos_wifi.img`
- `ROOTFS_IMG`：新 rootfs 路径，默认自动下载（`cargo xtask starry rootfs --arch riscv64`）
- `OUT_IMAGE`：新镜像路径，默认在源镜像同目录生成 `*_rootfs-YYYYMMDD-HHMMSS.img`

**说明**：`cp` 源镜像 → `dd` 写入新 rootfs 到新镜像的 ext4 分区。分区表和 boot 分区保持不变。

## 镜像布局

```
SD 卡镜像 (2.1 GB, MBR/DOS 分区表)
├── Sector 0-2047:     MBR + 预留空间 (1 MB)
├── Partition 1:       FAT32 64 MB (bootable, type 0x0C)
│   ├── fip.bin        Cvitek 一级 bootloader (441 KB)
│   └── boot.sd        FIT Image = kernel + ramdisk + DTB (~16 MB)
├── Partition 2:       ext4 2 GB (type 0x83)
│   └── Alpine Linux rootfs
```

**启动流程**：`ROM → fip.bin (FSBL + BL31 + U-Boot SPL) → boot.sd (FIT) → StarryOS kernel → /bin/sh`

## 关键分区参数

| 参数 | 值 |
|------|-----|
| Boot 分区起始扇区 | 2048 |
| Boot 分区大小 | 131072 扇区 (64 MB) |
| Rootfs 分区起始扇区 | 133120 |
| Rootfs 分区大小 | 4198400 扇区 (~2 GB) |
| 镜像总扇区 | 4333568 |
| StarryOS 加载地址 | `0x80200000` |

## 常见问题

### U-Boot 启动后 StarryOS 无输出

确认 DTB 来自项目（~24 KB），而非 Linux 镜像（~22 KB）。脚本已自动使用项目 DTB。

### boot.sd 验证

```bash
mkimage -l target/sg2002/boot.sd
```

### 镜像验证

```bash
cd target/sg2002
OUT="sg2002_starryos_wifi.img"
BOOT_OFF=$((2048 * 512))
mdir -i "${OUT}@@${BOOT_OFF}" ::
mcopy -i "${OUT}@@${BOOT_OFF}" ::boot.sd /tmp/check.sd && mkimage -l /tmp/check.sd
```
