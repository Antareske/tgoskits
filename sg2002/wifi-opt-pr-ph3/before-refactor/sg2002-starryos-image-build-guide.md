# SG2002 StarryOS Wi-Fi SD 卡镜像构建指南

基于 `sg2002/wifi` 分支，构建可在 LicheeRV Nano 上运行的 StarryOS Wi-Fi SD 卡镜像。

## 概述

SD 卡镜像由以下组件组成：

```
SD 卡镜像 (2.1 GB, MBR/DOS 分区表)
├── Sector 0-2047:  MBR + 预留空间 (1 MB)
├── Partition 1:    FAT32 64 MB (bootable, type 0x0C)
│   ├── fip.bin       Cvitek 一级 bootloader (441 KB)
│   └── boot.sd       FIT Image = kernel + ramdisk + DTB (~16 MB)
├── Partition 2:    ext4 2 GB (type 0x83)
│   └── Alpine Linux rootfs
```

**启动流程**：`ROM → fip.bin (FSBL + BL31 + U-Boot SPL) → boot.sd (FIT) → StarryOS kernel → /bin/sh (init)`

## 环境要求

| 工具 | 用途 | 安装 |
|------|------|------|
| `cargo xtask` | 编译 StarryOS kernel、下载 rootfs | 项目自带 |
| `mkimage` | 生成 FIT image | `apt install u-boot-tools` |
| `dumpimage` | 从已有 FIT image 提取组件 | `apt install u-boot-tools` |
| `mcopy` | 写入 FAT32 boot 分区 | `apt install mtools` |
| `mkfs.fat` | 格式化 FAT32 分区 | 系统自带 |
| `sfdisk` | 创建 MBR 分区表 | 系统自带 |
| `dd` | 读写原始磁盘数据 | 系统自带 |

所有命令从仓库根目录执行。

## 步骤一：编译 StarryOS 内核

Wi-Fi 配置是 `licheerv-nano-sg2002-wifi.toml`，但 quick-start 默认使用标准配置。编译前需临时替换：

```bash
# 从仓库根目录执行
cd os/StarryOS/configs/board/

# 备份默认配置
cp licheerv-nano-sg2002.toml licheerv-nano-sg2002.toml.bak
cp licheerv-nano-sg2002.its  licheerv-nano-sg2002.its.bak

# 替换为 Wi-Fi 配置
cp licheerv-nano-sg2002-wifi.toml licheerv-nano-sg2002.toml
cp licheerv-nano-sg2002-wifi.its  licheerv-nano-sg2002.its

# 回到仓库根目录编译
cd ../../../../
cargo xtask starry quick-start licheerv-nano-sg2002 build

# 恢复默认配置
cd os/StarryOS/configs/board/
mv licheerv-nano-sg2002.toml.bak licheerv-nano-sg2002.toml
mv licheerv-nano-sg2002.its.bak  licheerv-nano-sg2002.its
cd ../../../../
```

产出：`target/riscv64gc-unknown-none-elf/release/starryos.bin`（~13.6 MB），加载地址 `0x80200000`。

## 步骤二：准备 fip.bin 和 ramdisk.bin

`fip.bin`（440832 字节）和 `ramdisk.bin`（2527648 字节）是 Cvitek 平台固化的 bootloader 组件，不随 StarryOS 内核变化，获取一次即可复用。

**方式 A（推荐）：复用已有文件**

若之前成功构建过，`target/sg2002/` 下已有这两个文件，校验大小后直接使用：

```bash
stat -c%s target/sg2002/fip.bin       # 应为 440832
stat -c%s target/sg2002/ramdisk.bin   # 应为 2527648
```

**方式 B：从已有 boot.sd 提取 ramdisk**

如果已有 `fip.bin` 但缺少 `ramdisk.bin`（或想重建），可从已生成过的 `boot.sd` 中提取 ramdisk：

```bash
dumpimage -T flat_dt -p 1 -o target/sg2002/ramdisk.bin target/sg2002/boot.sd
```

**方式 C：从 SG2002 Linux 镜像提取**

如果以上都没有，从已知可用的 SG2002 Linux 镜像（如 `sipeed` 官方固件）提取。设 Linux 镜像为 `sg2002_linux.img`：

```bash
# 查找 FAT32 boot 分区偏移（Start 列为扇区号）
fdisk -l sg2002_linux.img
# 假设 Start = 2048，则字节偏移 = 2048 × 512 = 1048576

BOOT_OFF=$((2048 * 512))
mcopy -i "sg2002_linux.img@@${BOOT_OFF}" ::fip.bin target/sg2002/fip.bin
mcopy -i "sg2002_linux.img@@${BOOT_OFF}" ::boot.sd /tmp/linux_boot.sd
dumpimage -T flat_dt -p 1 -o target/sg2002/ramdisk.bin /tmp/linux_boot.sd
```

**注意**：`fdisk -l` 输出中 Start 列是第 3 列（非第 2 列），且不同系统 `fdisk` 格式可能有差异。建议直接目视确认扇形号再代入 `BOOT_OFF`。

## 步骤三：准备 DTB

**【重要】必须使用项目自带的 DTB，不能使用 Linux 镜像中的 DTB。**

```bash
cp os/StarryOS/configs/board/licheerv-nano-sg2002.dtb target/sg2002/
```

项目 DTB（~24 KB）与 StarryOS 内核驱动实现匹配。Linux 镜像中的 DTB 大小和内容不同，会导致启动失败。

## 步骤四：生成 FIT Image (boot.sd)

在 `target/sg2002/` 目录下集中所有 FIT 组件并生成。

首先确认所需文件均已就位：

```bash
ls -l target/sg2002/starryos.bin \
      target/sg2002/ramdisk.bin \
      target/sg2002/licheerv-nano-sg2002.dtb
```

若 `starryos.bin` 不在该目录，拷贝一份：

```bash
cp target/riscv64gc-unknown-none-elf/release/starryos.bin target/sg2002/
```

创建 `target/sg2002/boot.its`（`/incbin/` 路径相对于 its 文件所在目录）：

```dts
/dts-v1/;

/ {
    description = "StarryOS WiFi kernel for SG2002 LicheeRV Nano";
    #address-cells = <1>;

    images {
        kernel-1 {
            description = "StarryOS WiFi kernel";
            data = /incbin/("starryos.bin");
            type = "kernel";
            arch = "riscv";
            os = "linux";
            compression = "none";
            load = <0x80200000>;
            entry = <0x80200000>;
            hash-1 {
                algo = "crc32";
            };
        };

        ramdisk-1 {
            description = "cvitek ramdisk";
            data = /incbin/("ramdisk.bin");
            type = "ramdisk";
            arch = "riscv";
            os = "linux";
            compression = "none";
            hash-1 {
                algo = "crc32";
            };
        };

        fdt-sg2002_licheervnano_sd {
            description = "cvitek device tree - sg2002_licheervnano_sd";
            data = /incbin/("licheerv-nano-sg2002.dtb");
            type = "flat_dt";
            arch = "riscv";
            compression = "none";
            hash-1 {
                algo = "sha256";
            };
        };
    };

    configurations {
        default = "config-sg2002_licheervnano_sd";
        config-sg2002_licheervnano_sd {
            description = "StarryOS WiFi boot for sg2002_licheervnano_sd";
            kernel = "kernel-1";
            ramdisk = "ramdisk-1";
            fdt = "fdt-sg2002_licheervnano_sd";
        };
    };
};
```

生成 `boot.sd`：

```bash
cd target/sg2002
mkimage -f boot.its boot.sd
cd ../..
```

验证：

```bash
mkimage -l target/sg2002/boot.sd
```

产出：`target/sg2002/boot.sd`（~16 MB）。

## 步骤五：准备 Rootfs

```bash
cargo xtask starry rootfs --arch riscv64
```

产出：`tmp/axbuild/rootfs/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img`（2 GB ext4）。

若 `tmp/axbuild/rootfs/` 下已有该文件且未过期，xtask 会自动跳过下载。

## 步骤六：组装 SD 卡镜像

在 `target/sg2002/` 目录下执行：

```bash
cd target/sg2002

OUT="sg2002_starryos_wifi.img"

# 创建空镜像（2.1 GB）
dd if=/dev/zero of="$OUT" bs=512 count=0 seek=4333568

# 创建 MBR 分区表
sfdisk "$OUT" <<EOF
label: dos
unit: sectors
start=2048, size=131072, type=c, bootable
start=133120, size=4198400, type=83
EOF

# 格式化 boot 分区并写入 fip.bin + boot.sd
mkfs.fat -F 32 -n BOOT --offset 2048 "$OUT"
BOOT_OFF=$((2048 * 512))
mcopy -i "${OUT}@@${BOOT_OFF}" fip.bin ::
mcopy -i "${OUT}@@${BOOT_OFF}" boot.sd ::

# 写入 rootfs（dd 到 ext4 分区，无需预先格式化）
ROOTFS_IMG="../../tmp/axbuild/rootfs/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img"
dd if="$ROOTFS_IMG" of="$OUT" bs=512 seek=133120 conv=notrunc status=progress

cd ../..
```

产出：`target/sg2002/sg2002_starryos_wifi.img`（2.1 GB）。

## 步骤七：验证

```bash
cd target/sg2002
OUT="sg2002_starryos_wifi.img"
BOOT_OFF=$((2048 * 512))
PART_OFF=$((133120 * 512))

# 分区表
fdisk -l "$OUT"

# boot 分区内容
mdir -i "${OUT}@@${BOOT_OFF}" ::

# boot.sd FIT image 完整性
mcopy -i "${OUT}@@${BOOT_OFF}" ::boot.sd /tmp/check.sd
mkimage -l /tmp/check.sd

# rootfs 中 /bin/sh 存在
debugfs -R "stat /bin/sh" "${OUT}?offset=${PART_OFF}"

cd ../..
```

## 烧写

```bash
sudo dd if=target/sg2002/sg2002_starryos_wifi.img of=/dev/mmcblk0 bs=4M status=progress conv=fsync
```

串口连接：115200 8N1。

## 常见问题

### U-Boot 无法启动（串口无 U-Boot 输出）

- 确认 `fip.bin` 大小正确（440832 字节），且来源于 Cvitek SG2002 镜像
- 确认 SD 卡分区表正确：`fdisk -l` 检查 partition 1 起始扇区为 2048，类型为 FAT32 bootable

### U-Boot 启动后 StarryOS 不输出

- 确认 DTB 使用了项目自带的 `licheerv-nano-sg2002.dtb`（~24 KB），而非 Linux 镜像中的 DTB（~22 KB）

### StarryOS panic: "Failed to resolve executable path"

- 确认 ext4 分区已写入 rootfs（不能仅格式化后留空）
- 确认 `/bin/sh` 存在且是到 `/bin/busybox` 的符号链接

### boot.sd 生成注意事项

- ITS 中 kernel 使用 raw binary（`.bin`），不是 uImage（`.uimg`）
- kernel 加载地址和入口地址均为 `0x80200000`
- ramdisk `compression = "none"`（cvitek ramdisk 未压缩）
- ITS 中 `/incbin/` 路径相对于 its 文件所在目录，**不要用绝对路径**
