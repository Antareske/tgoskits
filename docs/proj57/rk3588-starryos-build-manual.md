# RK3588 StarryOS SMP8 整盘镜像构建文

本文说明如何为 Orange Pi 5 Plus（RK3588）构建一张可直接上电启动的 StarryOS 整盘 TF 卡镜像。镜像自带完整启动链（idbloader + U-Boot），从 TF 卡自主引导，板上只运行 StarryOS，不含 Linux 运行时。本文产物按 `smp=8` 命名为 `rk3588-starryos-smp8`。

背景：RK3588 开发板的 uboot 版本过旧（如 uboot 2017.09），不支持 tgoskits 的板测命令。一种方案是使用官方工具刷写 SPI 更新 uboot，一劳永逸但是过程较复杂；本方案则是直接构建带有新 uboot 和引导链的整盘 TF 卡镜像，由烧录好的镜像主导加载。

硬件准备：RK3588 开发板 & 电源、TF 卡、读卡器、CH340 USB 转 TTL 模块 (支持 1500000 速率的波特率)。

文件准备：较新的 Orange Pi 5 Plus 的整盘 Linux 镜像（本文使用 Armbian）、tgoskits 及其开发环境。

---

## 1. 环境准备

Linux 开发机或虚拟机（含交叉构建 StarryOS 的 tgoskits 工作区）。

安装以下工具：

```bash
apt install -y gdisk fdisk mtools dosfstools u-boot-tools \
               device-tree-compiler xz-utils
```

另需 `rust-objcopy`（来自 rustup 的 `llvm-tools` 或 `cargo-binutils`），用于把内核 ELF 转成裸二进制。

---

## 2. 资产准备

构建需要四类资产：启动链、设备树、StarryOS 内核、StarryOS 根文件系统。其中**启动链**取自一份现成的 Armbian 镜像，**设备树、内核、根文件系统**均由 tgoskits 工作区提供。

> 设备树必须使用 tgoskits 提供的官方 DTB `os/StarryOS/configs/board/orangepi-5-plus.dtb`，**不要**用 Armbian 镜像里的 DTB。原因：StarryOS 的 RKNPU 驱动只匹配 `compatible = "rockchip,rk3588-rknpu"`，而较新的 Armbian 内核（如 6.18）改用了三核拆分布局 `rockchip,rk3588-rknn-core`，与 StarryOS 驱动不匹配，会导致 NPU 设备无法 probe 注册（NPU 用例运行时所有 rknpu ioctl 返回 NotFound 并段错误）。官方 DTB 的 NPU 节点为单节点 `rockchip,rk3588-rknpu`，已验证可在板上正常驱动 NPU。

### 2.1 现成的 Armbian 镜像（启动链来源）

本文使用官方 Armbian 的 Orange Pi 5 Plus 镜像作为启动链（idbloader + u-boot.itb）的来源：https://armbian.com/boards/orangepi5-plus

```text
Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

它已适配 RK3588，自带可用的 idbloader、U-Boot（2026.01，对 StarryOS FIT 镜像支持完整）。任何同样适配 Orange Pi 5 Plus 的镜像应该均可替代。本文只从中提取启动链，设备树另取自 tgoskits（见 2.3）。

```bash
xz -dk Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz
```

### 2.2 启动链（idbloader + u-boot.itb）

RK3588 采用 Rockchip Boot Flow 2 布局：`idbloader.img`（DDR 初始化 + SPL）写在 **sector 64**，`u-boot.itb`（含 U-Boot + ATF/TEE 的 FIT）写在 **sector 16384**。从 Armbian 镜像按固定扇区提取：

```bash
ARMBIAN=Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img

# sector 64：idbloader，魔数应为 "RKNS"
dd if=$ARMBIAN bs=512 skip=64 count=1 | od -An -c | head -1

# sector 16384：u-boot.itb，魔数应为 d00dfeed
dd if=$ARMBIAN bs=512 skip=16384 count=1 | od -An -tx1 | head -1

# 提取
dd if=$ARMBIAN of=idbloader.img bs=512 skip=64    count=8128
dd if=$ARMBIAN of=u-boot.itb    bs=512 skip=16384 count=8192
```

### 2.3 设备树（DTB）

使用 tgoskits 工作区提供的官方 DTB 作为 StarryOS 设备树的基底：

```text
os/StarryOS/configs/board/orangepi-5-plus.dtb
```

它的 NPU 节点为 `compatible = "rockchip,rk3588-rknpu"`，与 StarryOS 的 RKNPU 驱动匹配，是 NPU 能在板上工作的前提。直接拷出作为基底，后续修改其 `/chosen/bootargs`（见 4.1）：

```bash
cp os/StarryOS/configs/board/orangepi-5-plus.dtb rk3588-orangepi-5-plus.dtb
```

> 切勿改用 Armbian 镜像内的 DTB（`/boot/dtb/rockchip/rk3588-orangepi-5-plus.dtb`）。较新 Armbian 内核的 NPU 节点是 `rockchip,rk3588-rknn-core` 三核布局，StarryOS 驱动不识别，NPU 无法初始化。

### 2.4 StarryOS 内核

内核的 SMP（CPU 核数）是**编译期**固化进二进制的，由板级构建配置 `os/StarryOS/configs/board/orangepi-5-plus.toml` 中的 `max_cpu_num` 决定。编译时该值会转成 `SMP` 环境变量并写入内核，运行时不可更改。本文使用八核：

```toml
# os/StarryOS/configs/board/orangepi-5-plus.toml
max_cpu_num = 8
plat_dyn = true
```

> 构建前核对缓存模板 `tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml` 的核数，与源配置不一致时删除，下次构建会按源配置重新生成：
>
> ```bash
> grep max_cpu_num tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
> rm -f tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
> ```

在 tgoskits 工作区根目录编译内核：

```bash
cargo xtask starry quick-start orangepi-5-plus build
```

内核 ELF 产物位于 `target/aarch64-unknown-linux-musl/release/starryos`，构建末尾已自动生成同目录的 `starryos.bin`。如需手动转换，在工作区根目录执行：

```bash
rust-objcopy -O binary \
  target/aarch64-unknown-linux-musl/release/starryos \
  starryos.bin
```

构建后校验 SMP 为八核：

```bash
grep max_cpu_num tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml   # 8
```

启动后串口 banner 的 `smp = 8` 即反映此编译值。

### 2.5 StarryOS 根文件系统

由 tgoskits 下载预构建的 aarch64 根文件系统镜像（ext4，含 StarryOS 用户态）：

```bash
cargo xtask starry rootfs --arch aarch64
```

产物路径见命令输出末尾的 `rootfs ready at ...` 行，位于 `/tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img`。拷出作为后文的 `starry-rootfs.ext4`：

```bash
cp /tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img starry-rootfs.ext4
```

---

## 3. 关键启动地址

RK3588 的 U-Boot 环境约定了内核相关的内存地址，FIT 与引导脚本必须与之对齐，否则内核跳转后会因落在非法/重叠内存而发生预期外的错误（e.g. 静默失败）：

| 地址 | 含义 |
| --- | --- |
| `0x02000000` | 内核最终运行地址（FIT 的 `load` / `entry`） |
| `0x0a000000` | FIT 文件 `fatload` 落地的临时地址（引导脚本 `fitaddr`） |
| `0xfeb50000` | RK3588 调试串口的物理 MMIO 基址（用于 `earlycon`） |

---

## 4. 构建启动组件

### 4.1 同步设备树命令行

StarryOS 在运行时从设备树 `/chosen/bootargs` 读取内核命令行，因此必须把基底 DTB 的 `bootargs` 改写为指向 Starry 根分区与正确串口：

```bash
dtc -I dtb -O dts rk3588-orangepi-5-plus.dtb -o starry.dts
```

官方 DTB 的 `/chosen` 节点形如（含 Linux 用的 bootargs 与 initrd 地址）：

```text
chosen {
    stdout-path = "serial2:1500000n8";
    linux,initrd-start = <...>;
    linux,initrd-end = <...>;
    bootargs = "root=UUID=... console=ttyS2,1500000 ... cma=128M ...";
};
```

改写为 StarryOS 用的形态：保留 `stdout-path`，把 `bootargs` 整体替换为指向 Starry 根分区的命令行，并删除 `linux,initrd-start/end`（StarryOS 不使用 initrd，残留地址可能误导内核）：

```text
chosen {
    stdout-path = "serial2:1500000n8";
    bootargs = "root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4";
};
```

然后回编：

```bash
dtc -I dts -O dtb starry.dts -o starry.dtb
```

各参数含义：

- `root=PARTLABEL=starry-rootfs`：以 GPT 分区标签指定根分区。
- `earlycon=uart8250,mmio32,0xfeb50000`：用物理 MMIO 形式指定早期串口，确保内核早期日志可见。
- `rootwait`：等待根分区设备就绪。
- `rootfstype=ext4`：明确根文件系统类型。

### 4.2 打包 FIT 镜像

编写 `starry.its`，内核加载/入口地址对齐 `0x02000000`：

```dts
/dts-v1/;
/ {
    description = "StarryOS for Orange Pi 5 Plus";
    #address-cells = <1>;
    images {
        kernel-1 {
            data = /incbin/("starryos.bin");
            type = "kernel"; arch = "arm64"; os = "linux";
            compression = "none";
            load  = <0x02000000>;
            entry = <0x02000000>;
            hash-1 { algo = "sha256"; };
        };
        fdt-1 {
            data = /incbin/("starry.dtb");
            type = "flat_dt"; arch = "arm64";
            compression = "none";
            hash-1 { algo = "sha256"; };
        };
    };
    configurations {
        default = "config-1";
        config-1 { kernel = "kernel-1"; fdt = "fdt-1"; };
    };
};
```

```bash
mkimage -f starry.its starry-image.fit
```

### 4.3 生成自动引导脚本

编写 `boot.cmd`，把 FIT 加载到临时地址后用 `bootm` 启动（使用 FIT 内置的设备树）：

```text
setenv fitaddr 0x0a000000
setenv bootargs root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4
for dev in 1 0; do
    if fatload mmc ${dev}:1 ${fitaddr} starry-image.fit; then
        bootm ${fitaddr}
    fi
done
```

> `for dev in 1 0` 同时兼容「有 eMMC（SD 为 mmc 1）」与「仅 SD（mmc 0）」两种设备编号。

编译为 U-Boot 脚本：

```bash
mkimage -A arm64 -T script -C none -n 'StarryOS RK3588 boot' \
        -d boot.cmd boot.scr
```

---

## 5. 制作整盘镜像

### 5.1 分区布局

镜像采用 Rockchip 标准单盘 GPT 布局：

```text
sector 64      : idbloader.img        (DDR init + SPL)
sector 16384   : u-boot.itb           (U-Boot + ATF/TEE)
p1 @ sector 32768  : FAT   "boot"          (112 MiB)
    ├── starry-image.fit
    ├── starry.dtb
    └── boot.scr
p2 @ sector 262144 : ext4  "starry-rootfs" (StarryOS 根文件系统)
```

### 5.2 创建镜像与分区表

```bash
IMG=rk3588-starryos-smp8.img

# 1.25 GiB 空镜像
truncate -s 1280M "$IMG"

# GPT：p1 FAT boot，p2 ext4 starry-rootfs
sgdisk -og "$IMG"
sgdisk -n 1:32768:262143  -t 1:0700 -c 1:boot          "$IMG"
sgdisk -n 2:262144:0      -t 2:8300 -c 2:starry-rootfs "$IMG"
```

> `sgdisk -n 2:262144:0` 会把 p2 拉伸到磁盘末尾。若希望分区与 ext4 文件系统大小对齐，可按 rootfs 实际扇区数指定 p2 终点（rootfs 字节数 ÷ 512 + 262144 − 1），或在写入后用 `resize2fs` 把文件系统扩展到占满分区。

### 5.3 写入启动链

```bash
dd if=idbloader.img of="$IMG" bs=512 seek=64    conv=notrunc
dd if=u-boot.itb    of="$IMG" bs=512 seek=16384 conv=notrunc
```

### 5.4 填充 FAT boot 分区

```bash
FAT_OFF=$((32768*512))

# 在镜像内偏移处创建 FAT 文件系统
mformat -i "$IMG@@$FAT_OFF" -F -v BOOT ::

mcopy -i "$IMG@@$FAT_OFF" starry-image.fit ::starry-image.fit
mcopy -i "$IMG@@$FAT_OFF" starry.dtb       ::starry.dtb
mcopy -i "$IMG@@$FAT_OFF" boot.scr         ::boot.scr
```

> p1 为 112 MiB。内核增大导致 FIT 接近该上限时，需相应增大 p1。

### 5.5 填充 starry-rootfs 分区

将 2.5 节由 tgoskits 下载的 StarryOS 根文件系统镜像（ext4）写入 p2（起始 sector 262144）：

```bash
dd if=starry-rootfs.ext4 \
   of="$IMG" bs=512 seek=262144 conv=notrunc
```

> 写入前确认 rootfs 不超过 p2 容量（p2 扇区数 = p2 终点 − 262144 + 1）。该镜像的卷为 ext4，已包含 StarryOS 所需的用户态（busybox 等）。写入后需确认 p2 的 GPT 标签为 `starry-rootfs`，与命令行 `root=PARTLABEL=starry-rootfs` 对应。

### 5.6 压缩（可选）

```bash
xz -T0 -6 -k "$IMG"
```

---

## 6. 校验

```bash
# SMP 八核
grep max_cpu_num tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml   # 8

# 分区与标签
sgdisk -p "$IMG"          # p1 boot / p2 starry-rootfs
sgdisk -i 2 "$IMG"        # Partition name: 'starry-rootfs'

# 启动链在位
dd if="$IMG" bs=512 skip=64    count=1 | od -An -tx1 | head -1   # 52 4b 4e 53 (RKNS)
dd if="$IMG" bs=512 skip=16384 count=1 | od -An -tx1 | head -1   # d0 0d fe ed

# FIT 加载地址
mkimage -l starry-image.fit                                      # Load/Entry: 0x02000000

# FAT 内容
mdir -i "$IMG@@$((32768*512))" ::
```

---

## 7. 烧录与启动

```bash
# 如已压缩则先解压
xz -dk rk3588-starryos-smp8.img.xz

sudo dd if=rk3588-starryos-smp8.img of=/dev/sdX bs=4M conv=fsync
sync
```

插卡上电（串口波特率 1500000），无需手动干预 U-Boot。启动流程为：

```text
idbloader → U-Boot → bootflow 扫描到 /boot.scr → fatload FIT 到 0x0a000000
        → bootm（内核运行于 0x02000000）→ 挂载 starry-rootfs → StarryOS shell
```

成功时串口最终出现：

```text
Welcome to Starry OS!
root@starry:/root #
```
