# RK3588 纯 StarryOS 整盘镜像复现报告

Orange Pi 5 Plus（RK3588）纯 StarryOS 整盘 TF 卡镜像的完整复现流程：从 Armbian 镜像提取启动链资产开始，到产出可直接上电启动的 Starry 镜像为止。

实测产物 `www/rk3588-pure-starryos-v3.img.xz` 已在真机启动成功，串口最终进入 `root@starry:/root #` 交互 shell（见 `www/boot3.log`）。

---

## 0. 目标与约束

- **诉求**：做一张 TF 卡，**像 Armbian 一样从 TF 卡加载新版 U-Boot**（而非依赖 RK3588 板载/SPI 里的老 U-Boot），TF 卡上只跑纯 StarryOS，不含 Linux 运行时。
- **方案形态**：单盘 GPT 镜像，自带 idbloader + u-boot.itb + FAT boot 分区 + 独立 `starry-rootfs` ext4 分区，靠 `boot.scr` 自动 `bootflow` 引导，无需手动敲 U-Boot 命令。
- **与官方双系统指南的区别**：`OrangePi5Plus_StarryOS_指南.md` 是「Orange Pi Linux 为主 + 手动进 Starry」，本报告是「TF 卡完全主导的纯 Starry 整盘镜像」。最关键的分叉点是 **FIT 加载地址**（见第 5 节）。

---

## 1. 环境与工具

Linux dev container，需以下工具：

```bash
apt install -y gdisk parted fdisk mtools dosfstools u-boot-tools \
               device-tree-compiler binwalk xz-utils
# rust-objcopy 来自 rustup 的 llvm-tools / cargo-binutils
```

涉及的输入文件：

| 文件 | 用途 |
| --- | --- |
| `www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz` | 启动链资产来源（idbloader / u-boot.itb / DTB） |
| `www/spi-uboot-backup.bin` | **仅作情报源**：从中提取板子真实内核加载地址，不写入镜像 |
| tgoskits 工作区 | 编译 StarryOS 内核 |

---

## 2. 从 Armbian 提取启动链资产

### 2.1 解压原始 Armbian 镜像

```bash
xz -dk www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz
```

### 2.2 确认原始布局

Armbian 原始镜像是 **GPT 单 `rootfs` 分区**，`/boot` 在 ext4 内，loader 写在 GPT 之前的保留扇区：

```text
First usable sector is 2048
Number  Start (sector)   End (sector)   Code  Name
   1          32768        3217407      8305  rootfs
```

启动链不在分区里，而在固定扇区（Rockchip Boot Flow 2 布局）：

```bash
# sector 64：idbloader（DDR init + SPL），魔数 "RKNS"
dd if=<armbian>.img bs=512 skip=64    count=1 | od -An -c | head -1
#   ->  R   K   N   S  \0 ...

# sector 16384：u-boot.itb（FIT，含 U-Boot + ATF/TEE），魔数 d00dfeed
dd if=<armbian>.img bs=512 skip=16384 count=1 | od -An -tx1 | head -1
#   ->  d0 0d fe ed ...
```

### 2.3 提取三类资产

```bash
A=target/rk3588-pure-starry/assets
mkdir -p "$A"

# idbloader：sector 64 起，约 2.5~4 MiB
dd if=<armbian>.img of="$A/idbloader.img" bs=512 skip=64 count=8128

# u-boot.itb：sector 16384 起，4 MiB
dd if=<armbian>.img of="$A/u-boot.itb"    bs=512 skip=16384 count=8192
```

DTB 从 Armbian 的 `/boot`（ext4 内）取得 `rk3588-orangepi-5-plus.dtb`，作为 Starry DTB 的基底。

提取产物校验：

| 资产 | 大小 | sha256（前 16） | 来源扇区 |
| --- | --- | --- | --- |
| `idbloader.img` | 4161536 B | `c4053ab5be308611…` | sector 64 |
| `u-boot.itb` | 4194304 B | `d7b7d2a982aaf348…` | sector 16384 |

> 串口里实际跑起来的 U-Boot 是 `U-Boot 2026.01_armbian`（见 `boot3.log:57`），来自 **TF 卡的 `u-boot.itb`**，证明「从 TF 卡加载新 U-Boot」诉求达成。

---

## 3. 编译 StarryOS 内核并转裸二进制

```bash
# tgoskits 工作区内编译 orangepi-5-plus 目标
cargo xtask starry quick-start orangepi-5-plus build
# 产物：target/aarch64-unknown-none-softfloat/release/starryos (ELF)

# ELF -> 裸 aarch64 Image
rust-objcopy -O binary \
  target/aarch64-unknown-none-softfloat/release/starryos \
  target/rk3588-pure-starry/assets/starryos-v3.bin
```

| 产物 | 大小 | sha256（前 16） |
| --- | --- | --- |
| `starryos-v3.bin` | 13242368 B | `f4931355f0436 1ba…` |

> 该哈希与 tgoskits `loady`/`run` 路径实测可启动的内核一致，保证镜像里固化的内核和实测内核是同一份二进制。

---

## 4. 从 SPI 备份提取板子真实加载地址（关键情报）

这是 v1/v2 失败、v3 成功的转折点。`spi-uboot-backup.bin` **不写入镜像**，仅用 `strings` 提取板子 U-Boot 环境变量里的真实地址：

```bash
strings -a www/spi-uboot-backup.bin | grep -iE 'kernel_addr_r|kernel_comp_addr_r|fdt_addr_r'
```

得到：

| 变量 | 值 | 用途 |
| --- | --- | --- |
| `kernel_addr_r` | `0x02000000` | 内核最终运行地址（FIT 的 `load`/`entry`） |
| `kernel_comp_addr_r` | `0x0a000000` | FIT 文件先 `fatload` 落地的临时地址 |
| `fdt_addr_r` | `0x12000000` | DTB 地址（FIT 内置 fdt 时未单独使用） |

> 官方指南用 `load=0x40000000`，那是配合 SPI 里 U-Boot 的环境。在 TF 卡自带 U-Boot 这条链路上 `0x40000000` 不成立，会卡在 `Starting kernel ...`。必须用板子 env 实际的 `0x02000000`。

---

## 5. 构建 Starry FIT 与启动脚本

### 5.1 同步 DTB 的 `/chosen/bootargs`

StarryOS 从 DTB 的 `/chosen/bootargs` 读取命令行，必须把 Armbian DTB 里残留的 `root=UUID=...` 改成 Starry 的根分区规格：

```bash
dtc -I dtb -O dts "$A/starry.dtb" -o /tmp/starry-v3.dts
# 将 chosen.bootargs 改为：
#   root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4
# 并删除无效的 linux,initrd-start/end
dtc -I dts -O dtb /tmp/starry-v3.dts -o "$A/starry-v3.dtb"
```

`earlycon=uart8250,mmio32,0xfeb50000` 是 RK3588 串口的物理 MMIO 形式（取自 tgoskits 实测配置），比 `console=ttyS2` 稳。

### 5.2 打 FIT（加载地址对齐板子 env）

`starry-v3.its` 关键字段：

```dts
kernel-1 {
    data = /incbin/("starryos-v3.bin");
    type = "kernel"; arch = "arm64"; os = "linux"; compression = "none";
    load  = <0x02000000>;   /* 板子 $kernel_addr_r，非指南的 0x40000000 */
    entry = <0x02000000>;
};
fdt-1 {
    data = /incbin/("starry-v3.dtb");
    type = "flat_dt"; arch = "arm64"; compression = "none";
};
```

```bash
mkimage -f "$A/starry-v3.its" "$A/starry-image-v3.fit"
```

| 产物 | 大小 | sha256（前 16） |
| --- | --- | --- |
| `starry-image-v3.fit` | 13523140 B | `a854f2f51e169e31…` |

### 5.3 生成自动引导脚本 `boot.scr`

`boot-v3.cmd`：

```text
setenv fitaddr 0x0a000000
setenv bootargs root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4
for dev in 1 0; do
    if fatload mmc ${dev}:1 ${fitaddr} starry-image.fit; then
        bootm ${fitaddr}
    fi
done
```

```bash
mkimage -A arm64 -T script -C none -n 'StarryOS RK3588 boot script v3' \
        -d "$A/boot-v3.cmd" "$A/boot-v3.scr"
```

`bootm` 不带 fdt 参数，直接用 FIT 内置的 fdt，路径与 tgoskits `loady`+`bootm` 等价。

---

## 6. 组装整盘镜像

镜像采用 Rockchip 标准布局（单盘 GPT）：

```text
sector 64      : idbloader.img        (RKNS,  DDR init + SPL)
sector 16384   : u-boot.itb           (d00dfeed, U-Boot 2026.01)
p1 @ sector 32768  : FAT  "boot"          112 MiB
    ├── starry-image.fit
    ├── starry.dtb
    └── boot.scr
p2 @ sector 262144 : ext4 "starry-rootfs"  1 GiB
```

写入 FAT boot 分区（FAT 偏移 = 32768 × 512）：

```bash
IMG=target/rk3588-pure-starry/rk3588-pure-starry-v3.img
FAT="$IMG@@$((32768*512))"

mcopy -o -i "$FAT" "$A/starry-image-v3.fit" ::starry-image.fit
mcopy -o -i "$FAT" "$A/starry-v3.dtb"        ::starry.dtb
mcopy -o -i "$FAT" "$A/boot-v3.scr"          ::boot.scr
```

`starry-rootfs` ext4 分区内是 Starry 的用户态根文件系统（alpine 基底 + busybox）。

压缩输出到 www：

```bash
xz -T0 -6 -k -c "$IMG" > www/rk3588-pure-starryos-v3.img.xz
xz -t www/rk3588-pure-starryos-v3.img.xz
```

| 产物 | 大小 |
| --- | --- |
| `rk3588-pure-starry-v3.img` | 1342177280 B (1.25 GiB) |
| `www/rk3588-pure-starryos-v3.img.xz` | 74233012 B (70.8 MiB) |

---

## 7. 校验清单

构建后逐项校验：

```bash
# GPT 分区与标签
sgdisk -p "$IMG"               # p1 boot / p2 starry-rootfs
sgdisk -i 2 "$IMG"             # Partition name: 'starry-rootfs'

# loader 在位
dd if="$IMG" bs=512 skip=64    count=1 | od -An -tx1 | head -1   # 52 4b 4e 53 (RKNS)
dd if="$IMG" bs=512 skip=16384 count=1 | od -An -tx1 | head -1   # d0 0d fe ed

# boot.scr / FIT 地址
mkimage -l "$A/starry-image-v3.fit"     # Load/Entry Address: 0x02000000
strings -a "$A/boot-v3.scr" | grep -E 'fitaddr|bootargs|bootm'
```

全部通过。

---

## 8. 烧录与启动

```bash
xz -dk www/rk3588-pure-starryos-v3.img.xz
sudo dd if=www/rk3588-pure-starryos-v3.img of=/dev/sdX bs=4M conv=fsync
sync
```

上电（串口 1500000，无需打断 autoboot），预期日志关键节点（对应 `www/boot3.log`）：

```text
U-Boot 2026.01_armbian ...                         # TF 卡自带新 U-Boot
Scanning for bootflows ... /boot.scr               # 自动 bootflow
## Loading kernel (any) from FIT Image at 0a000000 # FIT 落地临时地址
Loading Kernel Image to 2000000                    # 对齐 kernel_addr_r
Starting kernel ...
root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4
Debug serial : 0xfeb50000                          # 串口 console 生效
partition 2 name=Some("starry-rootfs") ... Ext4 filesystem mounted   # rootfs 挂载
Welcome to Starry OS!
root@starry:/root #                                # 进入交互 shell
```

> 实测 SMP=8 多核也成功进入 shell（`boot3.log:95` 初始化了 8 个 CPU），未触发官方指南 6.1 节描述的 stack guard TLB shootdown panic。

---

## 9. 成败关键三要素小结

按重要性排序，这三项是从「卡在 `Starting kernel ...`」到「进入 shell」的决定性配置：

1. **FIT 加载地址对齐板子 env**：`load=entry=0x02000000`（板子 `$kernel_addr_r`），**不是**指南的 `0x40000000`。这是 v1/v2 失败的根因。
2. **bootargs 用 RK3588 物理 MMIO 串口**：`earlycon=uart8250,mmio32,0xfeb50000`，而非 `console=ttyS2`，否则内核在跑但串口无输出。
3. **DTB 的 `/chosen/bootargs` 与 U-Boot env 同步**：Starry 从 DTB chosen 读 cmdline，必须把残留的 `root=UUID=...` 改成 `root=PARTLABEL=starry-rootfs`。

`spi-uboot-backup.bin` 全程**未写入镜像**，仅作为第 1、2 项参数的情报来源。
