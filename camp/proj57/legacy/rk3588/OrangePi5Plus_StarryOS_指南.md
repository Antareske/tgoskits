# Orange Pi 5 Plus 启动 StarryOS — 逐步操作指南

写给**第一次在这块板上跑 StarryOS**的人。本文**不依赖**任何私人部署脚本；只假设你已有：

- 一块 **Orange Pi 5 Plus（RK3588）**
- 一张已烧录 **官方 Orange Pi Linux 镜像** 的 TF/SD 卡（Debian/Ubuntu 均可）
- **SPI Flash 里已是 U-Boot 2025.04**（串口能看到 `U-Boot SPL 2025.04`；若仍是 2017.09，见 **附录 A / 附录 D** 刷 SPI）
- 一台 **x86_64 Linux PC**（Ubuntu 22.04/24.04），能编译 Rust
- **USB 转串口** 接板子调试口，波特率 **1500000**

StarryOS 源码来自上游 **[rcore-os/tgoskits](https://github.com/rcore-os/tgoskits)**，需自行克隆编译；**不能**只靠官方 Orange Pi 镜像里的文件启动 Starry。

---

## 0. 整体流程（先读一遍）

```text
PC 编译 Starry 内核 → 打成 FIT 镜像
        ↓
板子在 Orange Pi Linux：配 IP → adb connect → adb push 到 /boot 与 /root
        ↓
串口打断 U-Boot → fatload FIT/DTB → bootm
        ↓
出现 root@starry shell
```

**不会**改写官方 Linux 根分区里的系统文件；只是在 `/boot` 多放两个文件，需要时手动从 U-Boot 启动 Starry，平时仍可从 `boot.scr` 进 Orange Pi Linux。

---

## 1. TF/SD 卡上到底有什么（布局说明）

本节与 Rockchip 官方文档对齐，请先读：

- [Boot option](https://opensource.rock-chips.com/wiki_Boot_option) — 启动阶段、镜像打包、`dd` / `rkdeveloptool` 写法  
- [Partitions](https://opensource.rock-chips.com/wiki_Partitions) — GPT 默认扇区表（loader / boot / rootfs）

**重要：** 文档里的 `**0x40`、`0x4000` 等是 LBA 扇区号**（每扇区 512 字节），不是字节地址。  
换算：**字节偏移 = 扇区号 × 512**。  
`dd seek=N` 在 `**bs=512`** 时，`N` 就是扇区号（例如 `seek=64` = 扇区 `0x40`）。

### 1.1 五个启动阶段（Boot Stage）

Rockchip 把启动分成 5 段（Stage 1 在芯片 BootRom 里，不可改）：


| Stage | 名称                             | 典型程序                 | 镜像文件                                             | 扇区位置                               |
| ----- | ------------------------------ | -------------------- | ------------------------------------------------ | ---------------------------------- |
| 1     | Primary Program Loader         | BootRom              | —                                                | ROM                                |
| 2     | Secondary Program Loader (SPL) | TPL/SPL 或 miniloader | `**idbloader.img**`                              | `**0x40**`                         |
| 3     | —                              | U-Boot + ATF/TEE     | `**u-boot.itb**` 或 `**uboot.img` + `trust.img**` | `**0x4000**`（+ `**0x6000**` trust） |
| 4     | —                              | Linux 内核             | `**boot.img**` 或 GPT **boot** 分区                 | `**0x8000`**                       |
| 5     | —                              | 根文件系统                | `**rootfs.img**` 或 GPT **rootfs**                | `**0x40000`**                      |


**两种打包/刷写路径（官方称 Boot Flow 1 / 2）：**


|                  | Boot Flow 1（miniloader）                                     | Boot Flow 2（U-Boot TPL/SPL，**U-Boot 2025.04 属此类**）    |
| ---------------- | ----------------------------------------------------------- | ----------------------------------------------------- |
| Stage 2          | `idbloader.img` = ddr + **miniloader**                      | `idbloader.img` = **u-boot-tpl** + **u-boot-spl**     |
| Stage 3          | `**uboot.img`** @ `0x4000` + `**trust.img**` @ `**0x6000**` | `**u-boot.itb**` @ `0x4000`（FIT，已含 U-Boot + bl31/TEE） |
| Stage 3 trust 分区 | **需要** 单独 `trust.img`                                       | **不需要** 写 `0x6000`（已在 itb 内）                          |


### 1.2 GPT 默认扇区表（Rockchip 开源布局）

GPT 头在 **LBA 0～63**。下表摘自 [Partitions](https://opensource.rock-chips.com/wiki_Partitions)（与 Boot option 中 `dd seek=` 一致）：


| 分区名               | 起始扇区       | 十六进制        | 大小（约）   | GPT PartNum | 内容                               |
| ----------------- | ---------- | ----------- | ------- | ----------- | -------------------------------- |
| MBR + Primary GPT | 0          | —           | 32 KiB  | —           | 分区表                              |
| **loader1**       | **64**     | **0x40**    | 2.5 MiB | 1           | idbloader（miniloader 或 SPL）      |
| Vendor Storage    | 7168       | 0x1c00      | 256 KiB | —           | SN、MAC 等                         |
| U-Boot ENV        | 8128       | 0x1fc0      | 32 KiB  | —           | 环境变量                             |
| （reserved）        | …          | …           | …       | —           | 保留                               |
| **loader2**       | **16384**  | **0x4000**  | 4 MiB   | 2           | `uboot.img` 或 `**u-boot.itb`**   |
| **trust**         | **24576**  | **0x6000**  | 4 MiB   | 3           | `**trust.img`**（仅 miniloader 路径） |
| **boot**          | **32768**  | **0x8000**  | 112 MiB | 4           | 内核、dtb、extlinux 等（须 bootable）    |
| **rootfs**        | **262144** | **0x40000** | 剩余      | 5           | Linux 根文件系统                      |
| Secondary GPT     | 盘末尾        | —           | —       | —           | 备份 GPT                           |


官方说明（Partitions Note 1）：

- preloader 是 **miniloader** → loader2 放 `uboot.img`，trust 分区放 `trust.img`  
- preloader 是 **SPL + trust** → loader2 放 `**u-boot.itb`**，**trust 分区不用**

Orange Pi 官方整盘 `*.img` 基本按此 GPT 生成；Linux 里看到的 `**p1`/`p2` 往往对应 boot/rootfs 分区**，编号与 `mmcblk` 设备号仍须用 `lsblk` 确认（见 1.5 节）。

### 1.3 从 SD/TF 卡启动：官方 `dd` 命令

设备假定为 `/dev/sdb`（**必须改成你的 SD 设备**）。摘自 [Boot option § Boot from SD/TF Card](https://opensource.rock-chips.com/wiki_Boot_option#Boot_from_SD.2FTF_Card)：

**路径 A — with SPL（`u-boot.itb`，与 U-Boot 2025.04 一致）：**

```bash
sudo dd if=idbloader.img of=/dev/sdb bs=512 seek=64
sudo dd if=u-boot.itb    of=/dev/sdb bs=512 seek=16384
sudo dd if=boot.img      of=/dev/sdb bs=512 seek=32768
sudo dd if=rootfs.img    of=/dev/sdb bs=512 seek=262144
sync
```

**路径 B — with miniloader（旧 Orange Pi 2017 U-Boot 常见）：**

```bash
sudo dd if=idbloader.img of=/dev/sdb bs=512 seek=64
sudo dd if=uboot.img     of=/dev/sdb bs=512 seek=16384
sudo dd if=trust.img     of=/dev/sdb bs=512 seek=24576
sudo dd if=boot.img      of=/dev/sdb bs=512 seek=32768
sudo dd if=rootfs.img    of=/dev/sdb bs=512 seek=262144
sync
```

扇区与 `seek` 对照：


| 镜像                     | 扇区（hex） | `dd seek=`（bs=512） |
| ---------------------- | ------- | ------------------ |
| idbloader.img          | 0x40    | 64                 |
| uboot.img / u-boot.itb | 0x4000  | 16384              |
| trust.img              | 0x6000  | 24576              |
| boot.img               | 0x8000  | 32768              |
| rootfs.img             | 0x40000 | 262144             |


**已有官方 Orange Pi 整盘 img 时**：通常 **不必** 再手工 `dd` 上述文件；`dd if=Orangepi5plus_*.img of=/dev/mmcblkX` 已包含完整布局。只有**单独升级 U-Boot** 时才常用前两行（`idbloader` + `u-boot.itb`）。

### 1.4 `boot.img` 与 Orange Pi 的 `/boot`（Stage 4）

官方 `**boot.img`** 是把 **内核 Image/zImage + dtb**（及可选 extlinux）打成 **FAT 或 ext2** 镜像，刷到 **扇区 0x8000**。

Orange Pi 官方系统在 GPT **boot 分区**（同一扇区起点）上挂 `**/boot`**（vfat），内有：

- `boot.scr`、`Image`、`*.dtb`、`extlinux/`、`orangepiEnv.txt` 等

这与 Rockchip **Stage 4** 是同一物理区域的不同用法（整盘 img vs 单独 boot.img）。

**Starry 只需在此 FAT 分区追加两个文件**（不破坏原有文件）：

- `/boot/starry-image.fit`
- `/boot/starry.dtb`

### 1.5 从 SPI 启动 U-Boot 时（你的 2025.04 场景）

[Boot option](https://opensource.rock-chips.com/wiki_Boot_option) 说明：

> Boot from SPI flash means firmware for **stage 2 and 3** (SPL and U-Boot only) in SPI flash and **stage 4/5 in other place**.

因此：

- **SPI**：`idbloader` + `u-boot.itb`（Stage 2～3）  
- **SD**：仍承载 **Stage 4（boot）+ Stage 5（rootfs）**  
- SD 上 `0x40` / `0x4000` 处可能 **仍有** loader 副本，但上电 **不一定** 从 SD 读 Stage 2～3  
- 默认仍走 SD 上 `boot.scr` → **Orange Pi Linux**；进 Starry 需在 U-Boot 里 **手动** `fatload` + `bootm`（第 5 节）

### 1.6 Orange Pi 在 Linux / U-Boot 里看到的分区

`dd` 官方 img 后，**用户态**常见（有无 eMMC 会影响编号）：

```text
NAME        MOUNTPOINT   文件系统   典型内容
mmcblkXp1   /boot        vfat       boot.scr、Image、*.dtb、extlinux/ …
mmcblkXp2   /            ext4       Orange Pi 根文件系统（Starry 也会挂载）
```

从 SD 启动时，官方 wiki 提醒：若用 extlinux，需保证 `**root=**` 与真实块设备一致（例如 `root=/dev/mmcblk1p2`），见 [Boot option](https://opensource.rock-chips.com/wiki_Boot_option) 中 extlinux 示例。Starry 通过 DTB/内核自行解析根分区，一般 **不用** 改 Orange Pi 的 `extlinux.conf`。

### 1.7 Linux 设备名 vs U-Boot `mmc` 号（极易搞错）


| 场景                  | Linux 块设备       | U-Boot 命令     |
| ------------------- | --------------- | ------------- |
| **仅 SD，无 eMMC 模块**  | 常为 `mmcblk0`    | 常为 `mmc 0`    |
| **SD + 板载/eMMC 模块** | SD 常为 `mmcblk1` | SD 常为 `mmc 1` |


**不要死记表。** 在 **Orange Pi Linux 已启动** 时执行：

```bash
lsblk
findmnt /
```

示例输出：

```text
NAME        MAJ:MIN RM  SIZE MOUNTPOINT
mmcblk1     179:96   0 58.2G
├─mmcblk1p1 179:97   0 1024M /boot      ← FAT，放 FIT 的位置
└─mmcblk1p2 179:98   0 57.2G /          ← ext4 根分区
```

记下：

- **根分区** = `mmcblk1p2` → U-Boot 里对应 `**mmc 1:2`**
- **boot 分区** = `mmcblk1p1` → U-Boot 里对应 `**mmc 1:1`**

后文用 `**MMC_DEV=1**`、`**BOOT_PART=1**`、`**ROOT_PART=2**` 表示；请按你 `lsblk` 结果替换。

---

## 2. 在 PC 上编译 StarryOS 并打 FIT

### 2.1 安装依赖

```bash
sudo apt update
sudo apt install -y git curl build-essential python3 u-boot-tools device-tree-compiler
```

安装 Rust（tgoskits 仓库自带 `rust-toolchain.toml`，会拉 nightly）：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

安装 aarch64 musl 交叉编译器（Starry 裸机目标需要）：

```bash
mkdir -p ~/toolchain
curl -L -o /tmp/aarch64-linux-musl-cross.tgz \
  https://musl.cc/aarch64-linux-musl-cross.tgz
tar xf /tmp/aarch64-linux-musl-cross.tgz -C ~/toolchain
export PATH="$HOME/toolchain/aarch64-linux-musl-cross/bin:$PATH"
aarch64-linux-musl-gcc --version
```

### 2.2 克隆并配置（**必须单核**，否则真机会 panic）

```bash
git clone https://github.com/rcore-os/tgoskits.git
cd tgoskits
```

编辑板级配置，把 CPU 数改为 1（RK3588 上 SMP=8 会触发 stack guard page TLB shootdown panic，见第 6 节）：

```bash
sed -i 's/^max_cpu_num = .*/max_cpu_num = 1/' \
  os/StarryOS/configs/board/orangepi-5-plus.toml

grep max_cpu_num os/StarryOS/configs/board/orangepi-5-plus.toml
# 应显示：max_cpu_num = 1
```

若曾编译过，删除缓存配置，避免仍用旧的 SMP=8：

```bash
rm -f tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml
```

### 2.3 编译内核

```bash
export PATH="$HOME/toolchain/aarch64-linux-musl-cross/bin:$PATH"
cargo xtask starry quick-start orangepi-5-plus build
```

成功后应有：

```text
target/aarch64-unknown-none-softfloat/release/starryos.bin
os/StarryOS/configs/board/orangepi-5-plus.dtb
```

构建日志里应出现 `**SMP=1**`。若仍是 `SMP=8`，回到 2.2 删缓存后重编。

### 2.4 手工打 FIT（不依赖仓库脚本）

```bash
WORKDIR=/tmp/starry-fit-build
mkdir -p "$WORKDIR"
cp target/aarch64-unknown-none-softfloat/release/starryos.bin "$WORKDIR/"
cp os/StarryOS/configs/board/orangepi-5-plus.dtb "$WORKDIR/"

cat > "$WORKDIR/starry.its" << 'EOF'
/dts-v1/;

/ {
    description = "StarryOS for Orange Pi 5 Plus";
    #address-cells = <1>;

    images {
        kernel-1 {
            description = "StarryOS kernel";
            data = /incbin/("starryos.bin");
            type = "kernel";
            arch = "arm64";
            os = "linux";
            compression = "none";
            load = <0x40000000>;
            entry = <0x40000000>;
            hash-1 { algo = "sha256"; };
        };
        fdt-1 {
            description = "Orange Pi 5 Plus DTB";
            data = /incbin/("orangepi-5-plus.dtb");
            type = "flat_dt";
            arch = "arm64";
            compression = "none";
            hash-1 { algo = "sha256"; };
        };
    };

    configurations {
        default = "config-1";
        config-1 {
            description = "StarryOS boot";
            kernel = "kernel-1";
            fdt = "fdt-1";
        };
    };
};
EOF

cd "$WORKDIR"
mkimage -f starry.its image.fit

ls -lh image.fit
dumpimage -l image.fit
```

说明两个地址：


| 地址               | 含义                                                               |
| ---------------- | ---------------------------------------------------------------- |
| `**0x10000000**` | U-Boot 用 `fatload` 把 **整个 FIT 文件** 先读到 DRAM 的临时地址（下文 `loadaddr`） |
| `**0x40000000`** | FIT **内部** 声明的内核解压/运行地址（ITS 里 `load`/`entry`）                    |


把产物拷到固定位置，方便后面 `adb push`：

```bash
cp "$WORKDIR/image.fit" ~/starry-image.fit
cp os/StarryOS/configs/board/orangepi-5-plus.dtb ~/starry.dtb
```

---

## 3. 把 FIT 和 DTB 放到 SD 卡

**前提：板子当前在 Orange Pi Linux**（不是 U-Boot、不是 StarryOS）。传完后仍在 Linux 里，再按第 5 节进 U-Boot 启动 Starry。

Orange Pi 5 Plus 上 **Starry 没有 adbd**；传 FIT 必须在 **Linux 阶段**完成。下面 **方式 A（网络 adb）** 是 WSL + 网线的常用做法。

### 3.1 配网：板子 IP 与 `ip -br addr`

RTL8125 网口在 Orange Pi 5 Plus 上设备名常为 `**enP3p49s0`**（以你板子为准）。接网线后，在 **板子 shell**（串口或已有 adb）执行：

```bash
# 看网口名与是否已有 IP
ip -br addr

# 若没有 192.168.100.x，手动配（与 PC 同一网段；IP 可按你环境改）
sudo ip link set enP3p49s0 up
sudo ip addr add 192.168.100.2/24 dev enP3p49s0

# 再确认
ip -br addr show enP3p49s0
ping -c 2 192.168.100.1    # 网关或 PC 地址，可选
```

**PC / WSL 侧**也要能 ping 通板子，例如：

```bash
ping -c 2 192.168.100.2
```

若 WSL ping 不通、Windows 能 ping 通，需把 **Windows 主机网卡** 与板子配在同一网段，或为 WSL 配置镜像/桥接网络（否则 `adb connect` 会 `Connection refused`）。

没有键盘/SSH 时，只能 **串口** 登录 `orangepi` 执行上述 `ip addr add`（波特率 **1500000**）。

### 3.2 开启网络 adb（首次或 adb 离线时）

在 **WSL** 安装 adb：

```bash
sudo apt install -y adb
```

**路径 1 — 已有 USB adb（需 WSL 透传 USB）：** 在 **Windows PowerShell（管理员）**：

```powershell
winget install usbipd
usbipd list
usbipd bind --busid <BUSID>          # Orange Pi / Rockchip 或 adb 设备
usbipd attach --wsl --busid <BUSID>
```

WSL 里：

```bash
adb kill-server && adb start-server
adb devices                        # 应出现 device
adb tcpip 5555                     # 切到网络 adb
adb connect 192.168.100.2:5555
adb devices                        # 192.168.100.2:5555  device
```

之后可拔掉 USB，仅用网线 `adb connect`。

**路径 2 — 板子已开网络 adb：** 配好 IP 后直接：

```bash
adb connect 192.168.100.2:5555
adb devices
```

**串口 CH340** 若也要给 WSL 用，需单独 `usbipd attach` 串口 busid（与 adb USB 不是同一个设备）。传 FIT 用 **网络 adb** 即可，**不强制** USB adb。

### 3.3 方式 A：网络 adb push（推荐）

PC / WSL 上（FIT 路径按你实际修改）：

```bash
adb connect 192.168.100.2:5555
adb get-state                      # 应输出 device

adb push ~/starry-image.fit /boot/starry-image.fit
adb push ~/starry-image.fit /root/starry-image.fit
adb push ~/starry.dtb       /root/orangepi-5-plus.dtb
adb shell cp /root/orangepi-5-plus.dtb /boot/starry.dtb
adb shell sync
adb shell ls -lh /boot/starry-image.fit /boot/starry.dtb /root/starry-image.fit
```

若 `adb push` 到 `/boot` 报 Permission denied，在板子上用 sudo 拷贝（经 adb shell）：

```bash
adb push ~/starry-image.fit /tmp/starry-image.fit
adb push ~/starry.dtb       /tmp/starry.dtb
adb shell "echo orangepi | sudo -S cp /tmp/starry-image.fit /boot/starry-image.fit && \
           echo orangepi | sudo -S cp /tmp/starry.dtb /boot/starry.dtb && \
           echo orangepi | sudo -S cp /tmp/starry-image.fit /root/starry-image.fit && \
           echo orangepi | sudo -S cp /tmp/starry.dtb /root/orangepi-5-plus.dtb && sync"
```

说明：


| 路径                                                   | 用途                               |
| ---------------------------------------------------- | -------------------------------- |
| `/boot/starry-image.fit`、`/boot/starry.dtb`          | FAT，U-Boot `**fatload mmc X:1**` |
| `/root/starry-image.fit`、`/root/orangepi-5-plus.dtb` | ext4 备用，`**ext4load mmc X:2**`   |


传完后可 `**adb reboot**`，再在串口打断 autoboot（第 5 节）。

### 3.4 方式 B：scp（与 adb 二选一）

板子 IP 已配好、且 PC 能 `ssh orangepi@192.168.100.2` 时：

```bash
scp ~/starry-image.fit orangepi@192.168.100.2:~/
scp ~/starry.dtb           orangepi@192.168.100.2:~/
```

板子上：

```bash
sudo cp ~/starry-image.fit /boot/starry-image.fit
sudo cp ~/starry.dtb       /boot/starry.dtb
sudo cp ~/starry-image.fit /root/starry-image.fit
sudo cp ~/starry.dtb       /root/orangepi-5-plus.dtb
sync
```

### 3.5 方式 C：PC 读卡器直接写 FAT 分区

```bash
# 确认设备名，不要写错盘！
lsblk

sudo mount /dev/sdX1 /mnt   # X 换成你的读卡器设备
sudo cp ~/starry-image.fit /mnt/starry-image.fit
sudo cp ~/starry.dtb       /mnt/starry.dtb
sync
sudo umount /mnt
```

### 3.6 方式 D：板子上 wget（PC 开临时 HTTP 服务）

PC：

```bash
cd ~
python3 -m http.server 8000
```

板子：

```bash
wget http://<PC的IP>:8000/starry-image.fit -O /tmp/starry-image.fit
wget http://<PC的IP>:8000/starry.dtb -O /tmp/starry.dtb
sudo cp /tmp/starry-image.fit /boot/starry-image.fit
sudo cp /tmp/starry.dtb       /boot/starry.dtb
sync
```

---

## 4. 串口准备

### 4.1 接线

Orange Pi 5 Plus 调试串口在 **RTC 座附近 3Pin TTL**，接 USB 转串口：**GND、RX、TX**（板子 TX 接 USB RX，板子 RX 接 USB TX）。

### 4.2 PC 打开终端

```bash
sudo apt install minicom
sudo minicom -D /dev/ttyUSB0 -b 1500000
```

设备名可能是 `/dev/ttyUSB1`，以 `ls /dev/ttyUSB*` 为准。

退出 minicom：`Ctrl+A` 再按 `X`。

---

## 5. 从 U-Boot 启动 Starry（逐行命令）

### 5.1 重启并进入 U-Boot

板子在 Linux 下可：

```bash
sudo reboot
```

**立刻**看串口。出现 `Hit key to stop autoboot` 或倒计时 **0…3 秒** 时，**快速连按空格**（或按提示按其他键），直到停住，提示符类似：

```text
=>
```

（U-Boot 2025.04 用 `**=>**`；2017 旧版可能是 `orangepi5plus:` 或 `opi#`。）

### 5.2 确认 SD 在 U-Boot 里的编号

在 `=>` 后**一行一行**输入（每行回车）：

```text
mmc list
```

示例：

```text
mmc@fe2e0000: 1
mmc@fe2c0000: 0
```

再选 SD 对应设备（假设是 **1**，与上文 `MMC_DEV=1` 一致）：

```text
mmc dev 1
part list mmc 1
```

应能看到 **FAT 分区（type 0c/0b）** 和 **ext4 分区**。FAT 一般是 **分区 1**。

### 5.3 加载 FIT 与 DTB 并启动

仍在 `=>` 下，**逐行**执行：

```text
setenv loadaddr 0x10000000
fatload mmc 1:1 ${loadaddr} starry-image.fit
fatload mmc 1:1 ${fdt_addr_r} starry.dtb
bootm ${loadaddr} - ${fdt_addr_r}
```

把 `**1:1**` 换成你的 `**MMC_DEV:BOOT_PART**`（例如只有 SD 且无 eMMC 时可能是 `0:1`）。

**最后一行必须带 DTB**：`bootm ${loadaddr} - ${fdt_addr_r}`。  
不要只敲 `bootm ${loadaddr}`，旧 U-Boot 会解析失败。

### 5.4 若 `fatload` 报 File not found

改用 ext4 备用路径（分区号换成你的 `ROOT_PART`）：

```text
setenv loadaddr 0x10000000
ext4load mmc 1:2 ${loadaddr} /root/starry-image.fit
ext4load mmc 1:2 ${fdt_addr_r} /root/orangepi-5-plus.dtb
bootm ${loadaddr} - ${fdt_addr_r}
```

### 5.5 成功时应看到

```text
Welcome to Starry OS!
```

然后出现 shell：

```text
root@starry:/root #
```

可验证：

```text
uname -a
free -h
ls /
```

串口上内核日志和用户命令输出可能**交错在同一行**，属正常现象；看命令是否返回、exit code 是否为 0。

---

## 6. 常见问题

### 6.1 `task stack guard page TLB shootdown timeout`

- **原因**：`max_cpu_num > 1` 且 `plat_dyn = true` 时，SMP + stack guard 在 RK3588 真机不稳定。
- **处理**：按 **2.2 节** 设 `max_cpu_num = 1`，删 `tmp/axbuild/...` 缓存，**重新 build + 打 FIT + 复制到 SD**，再 boot。

### 6.2 `Please RESET the board`（bootm 立刻失败）

- **原因**：多为 **U-Boot 2017.09** 对 FIT 支持差；或 FIT 损坏。
- **处理**：确认 SPI 已是 **2025.04**；重新 `mkimage`；确认 `dumpimage -l image.fit` 正常。

### 6.3 `fatload` / `ext4load` 失败

- 用 **1.7 节** 在 Linux 里 `lsblk` 核对分区号。
- 在 U-Boot 里 `mmc dev N` + `part list mmc N` 对照。
- 确认文件名与 **3 节** 复制路径一致（`/boot/starry-image.fit`）。

### 6.4 打断 autoboot 失败，直接进了 Linux

- 重启后再试，**更早**按空格。
- 或在 Linux 里临时加长 U-Boot 等待（需 root）：

```bash
sudo apt install u-boot-tools   # 若无 fw_setenv
sudo fw_setenv bootdelay 5
sudo reboot
```

进 Starry 后可 `sudo fw_setenv bootdelay 0` 改回。

### 6.5 卡在 `optee check api`

- 多为 `**/boot/boot.scr` 丢失或损坏**，自动启动脚本没跑完。
- 在 U-Boot 手动 `boot.scr` 同路径检查；或从官方镜像重新提取 `/boot/boot.scr` 恢复。

### 6.6 能否用网线 TFTP 启动？

Orange Pi 5 Plus 的 **RTL8125 网卡**，出厂/旧 U-Boot **没有驱动**，`dhcp`/`tftp` 通常不可用。**请用 SD fatload 路径**，不要花时间配 TFTP。

---

## 7. 回到 Orange Pi Linux

- **不删** `/boot/boot.scr` 的情况下，**断电上电且不按键** 会照常进官方 Linux。
- 若曾改 `bootdelay` 或动过 `boot.scr`，在 Linux 下恢复官方 `boot.scr` 即可。

---

## 附录 A：SPI 仍是 U-Boot 2017.09 时

串口若显示 `U-Boot 2017.09-orangepi`，`bootm` Starry FIT 大概率失败。需升级到 **Boot Flow 2（SPL + `u-boot.itb`）**，例如 **U-Boot 2025.04**。

### A.1 文件从哪来

Orange Pi 官方 u-boot deb 解压后（见 [Boot option § Package option](https://opensource.rock-chips.com/wiki_Boot_option#Package_option)）：

```text
usr/lib/linux-u-boot-legacy-orangepi5plus_.../
├── idbloader.img
├── u-boot.itb      ← SPL 路径（Flow 2）
├── rkspi_loader.img
└── ...
```

### A.2 刷 SPI（MaskROM + MiniLoader）— 详见 **附录 D**

附录 A 只列结论；**MaskROM 进 Loader、WSL usbipd、`rkdeveloptool` 两套命令、常见卡死** 的逐步说明与踩坑在 **[附录 D](#附录-dmaskrom--miniloader-刷-spi-踩坑与逐步命令)**（当晚在此环节卡最久）。

简要流程（`rkdeveloptool` **1.0.x** 语法，与 Ubuntu apt / 本仓库 `MiniLoaderAll.bin` 一致）：

```bash
cd ~/rCore_leaning/rknn-sdk   # 或你放 MiniLoaderAll.bin 的目录

rkdeveloptool list             # MaskROM 时应看到 Rockchip 设备
rkdeveloptool boot MiniLoaderAll.bin
rkdeveloptool read 0 16777216 spi-before-flash.bin
rkdeveloptool write 0 spi-uboot-backup.bin
rkdeveloptool reset
```

**不要**用网上旧教程里的 `db` / `wl` / `rl` / `rd` / `ld`（`rkdeveloptool` 1.0.x 会报 invalid command）。  
**不要**把 16MB SPI 整盘镜像 `dd` 到 SD 卡。

也可在 Orange Pi Linux 里：`nand-sata-install` → **7**（SPI bootloader）。

### A.3 只更新 SD 卡上的 U-Boot（仍从 SD 启动）

官方 [Boot option § Boot from SD/TF Card](https://opensource.rock-chips.com/wiki_Boot_option#Boot_from_SD.2FTF_Card)，**SPL 路径**：

```bash
sudo dd if=idbloader.img of=/dev/mmcblkX bs=512 seek=64
sudo dd if=u-boot.itb    of=/dev/mmcblkX bs=512 seek=16384
sync
```

**miniloader 旧路径** 还需 `trust.img` @ `seek=24576`（见正文 **1.3 节路径 B**）。

eMMC 上若仍用 **旧版** `rkdeveloptool`（rockchip-linux，`db`/`wl` 语法），见 [Boot option](https://opensource.rock-chips.com/wiki_Boot_option#Boot_from_eMMC)：

```bash
rkdeveloptool db rkxx_loader_vx.xx.bin
rkdeveloptool wl 0x40   idbloader.img
rkdeveloptool wl 0x4000 u-boot.itb
rkdeveloptool rd
```

---

## 附录 D：MaskROM + MiniLoader 刷 SPI（踩坑与逐步命令）

> **是否需要：** 仅当你要把 **SPI 里的 U-Boot 换成 2025.04**（或救砖）才做本节。  
> **只跑 Starry、SPI 已是 2025.04** → 跳过，直接做第 3 节 `adb push` + 第 5 节 U-Boot 启动。  
> 参考：[Rockchip rkdeveloptool](https://opensource.rock-chips.com/wiki_Rkdeveloptool)、[Boot option § Boot from eMMC](https://opensource.rock-chips.com/wiki_Boot_option#Boot_from_eMMC)

### D.1 三种 USB，不要混

Orange Pi 5 Plus 上 PC 侧常见 **三个不同 USB 设备**，busid 每次可能变，以 `usbipd list` 为准：


| 用途                   | 典型 VID:PID              | 接法                               | 工具              |
| -------------------- | ----------------------- | -------------------------------- | --------------- |
| **adb**（传 FIT、RKNN）  | `2207:0000` Android ADB | Type-C 数据线                       | `adb`           |
| **MaskROM / Loader** | `2207:350b` Rockchip    | **MaskROM 模式** + Type-C OTG 到 PC | `rkdeveloptool` |
| **串口调试**             | `1a86:7523` CH340       | TTL 调试口                          | `minicom`       |


刷 SPI **必须**走 `**2207:350b` MaskROM 设备**，不是 adb 那个 `2207:0000`。  
串口 CH340 与 OTG **各 attach 一次**，busid 不同（例如串口 `3-1`、MaskROM `3-4`）。

### D.2 准备文件


| 文件                         | 说明                                                                                                              |
| -------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `**MiniLoaderAll.bin`**    | MaskROM 阶段下载到 RAM 的 Loader（本仓库 `rknn-sdk/MiniLoaderAll.bin`，或 Orange Pi / Rockchip 官方包里的 `rk3588_*loader*.bin`） |
| `**spi-uboot-backup.bin**` | 待写入的 **16MB SPI 整盘镜像**（含 U-Boot 2025.04；**不是** SD 卡 img）                                                        |
| `**spi-before-flash.bin`** | 刷前备份（同样 16MB = 16777216 字节）                                                                                     |


### D.3 安装 rkdeveloptool 并确认版本

```bash
sudo apt install -y rkdeveloptool usbutils
rkdeveloptool -v
# 期望：rkdeveloptool ver 1.0.0（或带 list / boot / read / write / reset 子命令的新 CLI）
```

**命令对照（踩坑最多）：**


| 旧教程 / rockchip-linux 版               | **1.0.x（当晚成功）**                            |
| ------------------------------------ | ------------------------------------------ |
| `rkdeveloptool ld`                   | `rkdeveloptool list`                       |
| `rkdeveloptool db MiniLoaderAll.bin` | `**rkdeveloptool boot MiniLoaderAll.bin`** |
| `rkdeveloptool rl 0 N file`          | `rkdeveloptool read 0 N file`              |
| `rkdeveloptool wl 0 file`            | `rkdeveloptool write 0 file`               |
| `rkdeveloptool rd`                   | `rkdeveloptool reset`                      |
| `cs`、`ul` 等                          | **不存在**，别抄旧博客                              |


若 `boot` 报 invalid，说明你装的是更老的 `db` 版；若 `db` 报 invalid，说明已是 1.0.x，应改用 `boot`。

### D.4 进入 MaskROM

1. **断电**板子。
2. **按住 MaskROM 键**（板子标注 MaskROM / RECOVERY 附近）。
3. 保持按住的同时 **上电**（或插入 Type-C 供电）。
4. **Type-C OTG** 接 PC（与仅 adb 时可能是同一口，但模式不同）。
5. 松开 MaskROM（部分板子需一直按住到 PC 识别，按官方手册为准）。

Windows **PowerShell（管理员）**：

```powershell
usbipd list
# 找 STATE 里 Rockchip / 2207:350b 那一行的 BUSID，例如 3-4

usbipd bind --busid 3-4 --force
usbipd attach --wsl --busid 3-4
```

若提示 `Unknown USB filter 'hrdevmon'`，加 `**--force**`。  
attach 成功后 Windows 列表里该设备常会 **消失**（已交给 WSL），属正常。

WSL 里确认：

```bash
lsusb | grep -i rockchip
# 或: lsusb -d 2207:350b

rkdeveloptool list
# 应列出设备；若 "Did not find any rockusb device" → 见 D.6
```

### D.5 刷 SPI 完整命令（逐行执行）

```bash
cd ~/rCore_leaning/rknn-sdk

# 1) 下载 MiniLoader 到 RAM，设备会从 MaskROM 变为 Loader 模式
rkdeveloptool boot MiniLoaderAll.bin

# 2) 备份整颗 SPI 16MB（强烈建议，救砖用）
rkdeveloptool read 0 16777216 spi-before-flash.bin
ls -lh spi-before-flash.bin    # 应约 16M

# 3) 写入新固件（16MB 整盘镜像）
rkdeveloptool write 0 spi-uboot-backup.bin

# 4) 复位，拔掉 MaskROM 线，正常上电
rkdeveloptool reset
```

`read` / `write` 第一个 `**0**` 是起始扇区/偏移（SPI 从 0 起）；`**16777216**` = 16×1024×1024 字节。  
成功时 `write`/`read` 会打印进度 **100%**。

### D.6 常见卡点（当晚实际遇到）


| 现象                                        | 原因                                            | 处理                                                                          |
| ----------------------------------------- | --------------------------------------------- | --------------------------------------------------------------------------- |
| `Did not find any rockusb device`         | 未进 MaskROM，或 usbipd 未 attach / attach 错 busid | 重新 MaskROM 上电；`usbipd attach` **350b** 那条，不是 adb `0000`                     |
| `boot MiniLoaderAll.bin` 后 `**list` 又空了** | Loader 起来后 USB **重新枚举**，WSL 里设备短暂消失           | **断电** → 再进 MaskROM → 重新 attach → 再 `boot`；或 Windows 先 detach 再 attach      |
| WSL `lsusb` 出现 `**0000:0002` 等怪 ID**      | usbipd + 重枚举异常                                | **断电板子** → `usbipd detach --busid …` → 重新 MaskROM → `bind --force` + attach |
| `db` / `wl` / `cs` **invalid command**    | 教程针对旧版 rkdeveloptool                          | 改用 `**boot` / `write` / `read` / `reset`**（D.3 对照表）                         |
| `boot` 一直无响应                              | attach 的是 adb 设备、或线只充电不传数据                    | 确认 **350b**；换 Type-C 线/口                                                    |
| 与 **minicom 串口** 冲突                       | 同一条 USB 被占                                    | MaskROM 阶段可不开 minicom；串口与 OTG 是不同设备                                         |
| 刷完起不来                                     | 镜像不对或写错存储                                     | 用 `**spi-before-flash.bin`** 写回：`write 0 spi-before-flash.bin`              |


### D.7 刷写后验证

1. **串口**（1500000）上电，应出现 `**U-Boot SPL 2025.04`**、`Trying to boot from SPI` 等。
2. 不按键时应仍能进 **Orange Pi Linux**（SD 上 `boot.scr` 未破坏）。
3. 再按第 3 节 **adb push** FIT，第 5 节 U-Boot `**fatload` + `bootm`** 进 Starry。

### D.8 不用 MaskROM 的替代

已在 **Orange Pi Linux** 且系统正常时，可用官方 `**nand-sata-install`** → **7 Install/Update the bootloader on SPI Flash**，由脚本写 SPI，无需 PC 上 `rkdeveloptool`（但仍需了解 SPI 与 SD 布局区别）。

---

## 附录 B：命令速查（替换 mmc 号后可直接复制）

```text
mmc dev 1
setenv loadaddr 0x10000000
fatload mmc 1:1 ${loadaddr} starry-image.fit
fatload mmc 1:1 ${fdt_addr_r} starry.dtb
bootm ${loadaddr} - ${fdt_addr_r}
```

---

## 附录 C：与私人笔记的区别

- 本文：**通用逐步命令**，对齐 [Rockchip Boot option](https://opensource.rock-chips.com/wiki_Boot_option) / [Partitions](https://opensource.rock-chips.com/wiki_Partitions)。
- 同目录 `STARRY_BOOT_NOTES.md`：当晚调试流水账，含 WSL/usbipd、adb 等私人环境细节，**不必**给其他人看。

上游跟踪：[tgoskits #580](https://github.com/rcore-os/tgoskits/issues/580)（Orange Pi 板级）、[#1179](https://github.com/rcore-os/tgoskits/issues/1179)（SMP stack guard panic）。