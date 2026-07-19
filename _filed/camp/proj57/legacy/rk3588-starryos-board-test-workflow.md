# RK3588 StarryOS 板测全链路方案

## 目标

本文整理 Orange Pi 5 Plus / RK3588 上 StarryOS 的可维护板测工作链。目标不是复刻 QEMU 的 per-case rootfs 镜像注入，而是在真实板子上减少拆卡和整卡重刷次数：

- StarryOS 内核镜像始终由 U-Boot 通过 `loady` 或 TFTP 加载。
- TF 卡不保存 StarryOS 内核/FIT 等启动资产。
- TF 卡只持久保存一个 StarryOS 专用 rootfs 分区。
- StarryOS rootfs 使用 tgoskits 管理的 `rootfs-aarch64-alpine.img`。
- 用户态测试资产在 StarryOS 已启动后，通过串口或网络在线上传。
- 初始 TF 卡准备和 rootfs 重置分别作为独立脚本/命令实现。

## 背景判断

### 与手工 Orange Pi 指南的区别

`www/OrangePi5Plus_StarryOS_指南.md` 的主流程是：先进入 Orange Pi Linux，通过 `adb`/`scp`/读卡器把 `starry-image.fit` 和 DTB 放到 `/boot`，再在 U-Boot 里 `fatload` 后 `bootm`。这适合一次性手工验证，但不适合作为 tgoskits 板测常态流程。

本方案保留其中可靠的部分：

- RK3588 的 boot 分区通常是 FAT/vfat。
- U-Boot 2025.04 更适合加载 Starry FIT。
- Linux 分区编号和 U-Boot `mmc` 号必须实测确认。
- U-Boot TFTP 在 RTL8125 驱动不可用时不可作为默认路径。

本方案刻意不采用其中的部分假设：

- 不要求板子 Linux 是 GUI 版。
- 不依赖 `adb`。
- 不把 Starry FIT/DTB 固化到 TF 卡 `/boot`。
- 不把 Armbian/Ubuntu rootfs 当作 StarryOS 的默认 rootfs。

### 与 SG2002 工作内容的关系

`www/tgoskits/doc/sg2002-starryos-image-guide.md` 提供了一个可借鉴的分层思路：boot 分区保存平台启动文件，rootfs 分区保存 Alpine Linux rootfs。SG2002 是整卡镜像拼装模型，而 RK3588 本方案是“保留已有 Linux 镜像 + 增加 Starry rootfs 分区 + U-Boot 动态加载内核”。

可复用的原则：

- rootfs 分区使用 ext4。
- rootfs 内容来自 tgoskits managed rootfs。
- boot 资产和 rootfs 资产分层管理。

不直接复用的部分：

- SG2002 镜像把 `boot.sd` 写入 FAT 分区；RK3588 日常板测不把 Starry FIT 持久写入 TF 卡。
- SG2002 是离线整卡镜像；RK3588 需要保留一张可回退进 Linux 的 TF 卡。

## StarryOS Rootfs 选择规则

StarryOS 的 rootfs 选择不是由 U-Boot 自动挂载完成。U-Boot 只负责把内核/FIT 加载到内存并执行；StarryOS 启动后根据 bootargs 里的 `root=` 选择块设备分区。

当前源码支持的显式 `root=` 写法包括：

- `root=/dev/mmcblkXpY`
- `root=/dev/sdXY`
- `root=PARTUUID=...`
- `root=PARTLABEL=...`

当前源码不支持 `root=UUID=...`。如果 U-Boot 原 Linux bootargs 里只有 `root=UUID=...`，StarryOS 会忽略这个 root 指定，然后走默认候选选择。

默认候选选择存在不确定性：

- 如果恰好只有一个名为 `rootfs` 的分区，会 fallback 到 `PARTLABEL=rootfs`。
- 如果只有一个可识别文件系统分区，也可能被选中。
- 如果 Linux rootfs 是唯一 ext4 分区，StarryOS 可能会误挂 Linux rootfs。

因此本方案要求新建 StarryOS 专用分区，并显式传入：

```text
root=PARTLABEL=starry-rootfs
```

## TF 卡目标布局

推荐 TF 卡保留已有 Linux 镜像，只在尾部新增 StarryOS 专用分区：

```text
TF card
├── 原 Linux boot 分区
│   └── 保留 Armbian/Ubuntu 自带 boot.scr、Image、dtb 等
├── 原 Linux rootfs 分区
│   └── 保留可回退维护环境
└── StarryOS rootfs 分区
    ├── 文件系统：ext4
    ├── 分区标签：PARTLABEL=starry-rootfs
    └── 内容：rootfs-aarch64-alpine.img
```

如果原 Linux rootfs 已占满整卡，需要先缩小 Linux rootfs 文件系统和分区。该操作有数据损坏风险，必须做成带有强校验和确认提示的脚本，不能作为静默自动步骤。

## Rootfs 来源

tgoskits 对 aarch64 的默认 managed rootfs 是：

```text
tmp/axbuild/rootfs/rootfs-aarch64-alpine.img
```

它来自 `rcore-os/tgosimages` release `v0.0.5`，归档 URL 形如：

```text
https://github.com/rcore-os/tgosimages/releases/download/v0.0.5/rootfs-aarch64-alpine.img.tar.xz
```

该 rootfs 是 Alpine 体系，不是 Debian/Ubuntu/Armbian。RK3588 是 aarch64，因此应使用 `rootfs-aarch64-alpine.img` 作为 StarryOS 板测基础 rootfs。

## 分阶段工作流

### 阶段 0：前置条件确认

需要确认：

- TF 卡已经烧录可启动的 Linux 镜像。
- Orange Pi 5 Plus 已能启动到板上 Linux，例如 Armbian minimal。
- U-Boot 支持 `loady`，可通过串口进入 U-Boot 命令行。
- 若计划使用 TFTP，U-Boot 必须能识别并驱动 RK3588 板上的网卡；当前 RTL8125 路径不应默认假设可用。
- 开发容器能访问串口，当前 Orange Pi 常用 `/dev/ttyUSB0`，波特率 `1500000`。
- PC 以太网 3 与板子直连，PC 地址为 `192.168.100.100/24`，板子有线网口建议固定为 `192.168.100.101/24`。
- 当前 RK3588 SMP 仍建议使用 `max_cpu_num = 1`，避免已知 `task stack guard page TLB shootdown timeout`。

串口登录建议命令：

```bash
picocom -b 1500000 /dev/ttyUSB0
```

登录用户可以是普通用户 `orangepi`，但所有分区和 rootfs 写入操作都必须切到 root：

```bash
sudo -i
```

原因是 `parted`、`fdisk`、`resize2fs`、`mkfs.ext4`、`dd`、`mount`、`losetup` 都需要 root 权限。分区调整过程不建议混用普通 shell 和零散 `sudo`，否则失败点不清晰。

### 阶段 1：准备 managed rootfs

开发机侧执行：

```bash
cargo xtask starry rootfs --arch aarch64
```

预期产物：

```text
tmp/axbuild/rootfs/rootfs-aarch64-alpine.img
```

该步骤只准备基础 Alpine rootfs，不注入任何测试用例。

### 阶段 2：板上 Linux 自修改 TF 卡

WSL 无法读取 TF 读卡器时，不再要求把 TF 卡插到 PC/WSL/容器里改分区。改为让 Orange Pi 5 Plus 先启动到板上的 Armbian minimal，再由板子自己修改正在使用的 TF 卡。

该策略分两种情况：

- 如果 TF 卡尾部已有未分配空间，可以在当前运行的 Armbian 中直接新增 `starry-rootfs` 分区。
- 如果 Armbian rootfs 已占满整卡，不能在当前已挂载的 `/` 上安全在线缩小 rootfs。需要改用 initramfs/rescue、USB 临时系统、或重新烧录时预留空间。

先在板上 Linux 里采集布局，必须用 root shell：

```bash
sudo -i
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,UUID,PARTUUID,MOUNTPOINTS
findmnt /
fdisk -l
parted -l
df -h /
```

重点判断：

- TF 卡设备名通常是 `/dev/mmcblk0` 或 `/dev/mmcblk1`，不能假设固定。
- 当前 `/` 所在分区不能被卸载，不能在线缩小。
- 若 `parted` 显示卡尾有足够 `Free Space`，可以只新增分区，不碰 Linux rootfs。
- 新分区必须设置 `PARTLABEL=starry-rootfs`。
- 新分区文件系统必须是 ext4。

板上网络准备建议先完成，方便从开发机/WSL/Windows 传 rootfs 镜像到板子：

```bash
sudo ip addr flush dev eth0
sudo ip addr add 192.168.100.101/24 dev eth0
sudo ip link set eth0 up
ping -c 3 192.168.100.100
```

如果网卡名不是 `eth0`，先用以下命令确认实际网卡名：

```bash
nmcli device status
ip link
```

持久配置建议用 NetworkManager，不设置默认网关，避免影响其它网络路径：

```bash
sudo nmcli con add type ethernet ifname eth0 con-name opi-direct ipv4.method manual ipv4.addresses 192.168.100.101/24 autoconnect yes
sudo nmcli con up opi-direct
```

当前网络拓扑推荐这样处理：

```text
Orange Pi Linux: 192.168.100.101/24
Windows Ethernet 3: 192.168.100.100/24
Docker dev container: 192.168.65.x 或其它 Docker 内部网段
```

板子优先验证能 ping 通 Windows：

```bash
ping -c 3 192.168.100.100
```

不要把“板子能 ping 通容器 IP”作为准备工作的硬要求。Docker Desktop/WSL2 常见 NAT 拓扑下，板子无法直接路由到容器内部网段是正常现象。

文件传输优先级：

1. 从 WSL 或容器主动 `scp` 到板子 `192.168.100.101`。
2. 如果容器不能直接访问板子，改由 WSL 或 Windows 执行 `scp`。
3. 如果必须让板子主动下载容器文件，用 Windows `netsh interface portproxy` 把 `192.168.100.100:<port>` 转发到容器 IP。

例如容器中起 HTTP 服务后，在 Windows 管理员 PowerShell 添加端口转发：

```powershell
netsh interface portproxy add v4tov4 listenaddress=192.168.100.100 listenport=8000 connectaddress=<container-ip> connectport=8000
```

板子下载：

```bash
wget http://192.168.100.100:8000/rootfs-aarch64-alpine.img
```

端口转发只处理 TCP，不处理 ICMP，所以不能用 ping 验证 portproxy。

如果确认卡尾已有空闲空间，最小手工流程如下，分区号和设备名必须按实际输出替换：

```bash
sudo -i
parted /dev/mmcblk0 print free
parted /dev/mmcblk0 mkpart starry-rootfs ext4 <start> <end>
partprobe /dev/mmcblk0
dd if=/root/rootfs-aarch64-alpine.img of=/dev/mmcblk0pN bs=4M conv=fsync status=progress
e2fsck -f /dev/mmcblk0pN
resize2fs /dev/mmcblk0pN
tune2fs -L starry-rootfs /dev/mmcblk0pN
lsblk -o NAME,SIZE,FSTYPE,LABEL,PARTLABEL,UUID,PARTUUID,MOUNTPOINTS
```

注意 `tune2fs -L starry-rootfs` 设置的是文件系统 label；StarryOS 启动选择依赖的是分区表里的 `PARTLABEL=starry-rootfs`。用 `parted mkpart starry-rootfs ...` 创建 GPT 分区时会设置分区名。最终必须用 `lsblk` 或 `blkid` 同时确认 label 和 PARTLABEL。

新增脚本建议命名为：

```text
scripts/rk3588-board-prepare-starry-rootfs.sh
```

输入参数建议：

- `--device /dev/mmcblk0`：板上 Linux 看到的 TF 卡块设备。
- `--rootfs /root/rootfs-aarch64-alpine.img`：已经上传到板上的 Starry rootfs 镜像。
- `--size 4G`：为 Starry rootfs 腾出的分区大小，默认可设为 4G 或 8G。
- `--label starry-rootfs`：新分区 PARTLABEL。
- `--dry-run`：只分析布局，不写盘。
- `--force`：确认执行破坏性操作。

脚本行为：

1. 检查设备是否是可移动盘或用户明确确认的块设备。
2. 输出 `lsblk -f`、分区表、文件系统类型、起止扇区、剩余空间。
3. 判断是否已有 `PARTLABEL=starry-rootfs`。
4. 如果已有足够未分配空间，直接新建 ext4 分区。
5. 如果没有未分配空间，停止并提示不能在线缩小当前挂载的 Linux rootfs。
6. 如果用户明确切换到 rescue/USB 临时系统，再允许缩小 Linux rootfs。
7. 新建 Starry 分区，设置 `PARTLABEL=starry-rootfs`。
8. 将 `rootfs-aarch64-alpine.img` 写入该分区，或格式化后复制镜像内容。
9. 运行校验，确认分区能被识别为 ext4，PARTLABEL 正确。

写入 rootfs 有两种可选实现：

- 镜像直写：`dd if=/root/rootfs-aarch64-alpine.img of=/dev/mmcblk0pN conv=fsync`。要求分区大小不小于镜像大小，写完后可按需扩展 ext4。
- 内容复制：挂载 rootfs 镜像和目标分区，把文件复制过去。需要 loop/mount 权限，适合需要扩容或保留目标分区 UUID 的场景。

脚本第一版应优先运行在板上 Armbian minimal 中，而不是 WSL/容器中。开发机只负责编译和传输 rootfs 镜像，不直接操作 TF 卡块设备。

### 阶段 3：日常构建和加载 StarryOS 内核

开发机侧构建：

```bash
cargo xtask starry quick-start orangepi-5-plus build
```

运行时仍沿用 quick-start 的 U-Boot 加载模式：

```bash
cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0
```

但需要让 U-Boot bootargs 包含：

```text
root=PARTLABEL=starry-rootfs
```

建议新增或扩展配置能力：

- 在 `orangepi-5-plus-uboot.toml` 中支持追加 bootargs。
- 或在 quick-start 生成 U-Boot 命令时追加 `setenv bootargs ... root=PARTLABEL=starry-rootfs`。
- 或提供 `--root PARTLABEL=starry-rootfs` 这类 CLI 参数。

如果仍由用户手动输入 U-Boot 命令，示例应类似：

```text
setenv bootargs root=PARTLABEL=starry-rootfs
loady ${loadaddr}
bootm ${loadaddr} - ${fdt_addr_r}
```

实际命令需要与 ostool 生成的 FIT/DTB 加载流程对齐。关键点是：内核镜像通过 `loady` 或 TFTP 进入内存，rootfs 只通过 bootargs 指向 TF 卡分区。

### 阶段 4：StarryOS 启动后在线上传测试资产

这是日常调试的核心路径。基础 rootfs 不随每个 case 重刷，测试文件通过用户态传输进入运行中的 StarryOS。

传输通道按优先级分为：

1. 网络上传：如果 StarryOS 上 RK3588 网卡驱动和用户态网络工具可用，优先通过网线传输。
2. 串口上传：网络不可用时，通过串口协议传输小文件或 tar 包。
3. Linux 中转：重启回 Armbian/Ubuntu Linux，通过 SSH/SCP 更新 `starry-rootfs` 或测试数据分区，再重新进入 U-Boot 启动 StarryOS。

推荐上传目标目录：

```text
/tmp/starry-cases
```

或持久目录：

```text
/root/starry-cases
```

如果 rootfs 写入频繁会影响稳定性，可新增单独 `testdata` 分区，由 StarryOS 启动后挂载到：

```text
/mnt/testdata
```

这样 rootfs 分区只作为稳定系统底座，测试资产放在可清理的数据分区。

### 阶段 5：执行和判定

tgoskits 板测命令需要在 StarryOS shell 出现后执行：

1. 等待 shell prompt，例如 `root@starry:`。
2. 上传测试资产。
3. 设置可执行权限。
4. 执行测试入口。
5. 通过 success/fail regex 判定结果。
6. 收集串口日志。

可将 QEMU grouped runner 的思路迁移到板测：上传一个统一 runner，例如：

```text
/usr/bin/starry-run-board-case-tests
```

runner 输出结构化标记：

```text
STARRY_BOARD_TEST_BEGIN: <name>
STARRY_BOARD_TEST_PASSED: <name>
STARRY_BOARD_TEST_FAILED: <name> status=<code>
STARRY_BOARD_TESTS_PASSED
STARRY_BOARD_TESTS_FAILED
```

这样板测和 QEMU 测试可以复用相似的日志判定模型。

## QEMU Overlay 与板测在线上传的关系

QEMU 当前 overlay 机制是启动前镜像注入：

```text
base rootfs -> per-case rootfs copy -> debugfs inject overlay -> QEMU boot
```

它不是运行时动态 overlay mount。`debugfs` 是宿主端修改 ext4 镜像的工具，overlay 是待注入的目录树，最终产物是一个带有测试资产的 rootfs 镜像副本。

真实板测不适合每个 case 都生成 rootfs 并拆卡写盘。因此板测应采用在线上传：

```text
base rootfs on TF -> boot StarryOS -> upload test assets -> run tests
```

可复用 QEMU 资产管线的部分：

- C/Rust 交叉编译。
- Shell/Python 资产收集。
- grouped runner 生成。
- ELF 运行时依赖扫描和补齐。

不应直接复用的部分：

- 每 case 复制完整 rootfs。
- 每 case 用 `debugfs` 注入 rootfs 镜像。
- 每 case 将完整 rootfs 写回 TF 卡。

## 需要新增的 tgoskits 能力

### TF 卡准备命令

建议新增：

```bash
cargo xtask starry board-rootfs prepare orangepi-5-plus --device /dev/sdX --size 4G
```

职责：分析和调整 TF 卡分区，创建 `PARTLABEL=starry-rootfs`，写入 `rootfs-aarch64-alpine.img`。

### TF 卡 rootfs 重置命令

建议新增：

```bash
cargo xtask starry board-rootfs reset orangepi-5-plus --device /dev/sdX
```

职责：在已存在 `starry-rootfs` 分区时，重新写入干净 rootfs。该命令不碰 Linux boot/rootfs 分区。

### Quick-start root 指定

建议扩展：

```bash
cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0 --root PARTLABEL=starry-rootfs
```

职责：把 `root=PARTLABEL=starry-rootfs` 注入 U-Boot bootargs。

### 板测资产上传命令

建议新增：

```bash
cargo xtask starry board-case upload --serial /dev/ttyUSB0 --case <case>
```

或：

```bash
cargo xtask starry board-case run --board orangepi-5-plus --case <case> --serial /dev/ttyUSB0
```

职责：构建/收集用户态资产，等待 StarryOS shell，上传资产，执行 runner，判定日志。

### 网络上传后端

网络可用时，上传后端可以从串口切换为 TCP/HTTP/SCP 类路径。但这依赖 StarryOS 的网卡驱动和用户态工具，不应作为第一阶段强依赖。

## 风险和处理

### 缩小 Linux rootfs 风险

缩小已有 Linux rootfs 是最高风险步骤。尤其当 Armbian 正从 TF 卡 rootfs 运行时，当前挂载为 `/` 的分区不能在线安全缩小。板上自修改策略第一版只允许使用卡尾未分配空间新增分区，不在在线系统中缩小 rootfs。

脚本必须默认 `--dry-run`，执行前展示：

- 设备路径。
- 原分区表。
- 新分区表。
- 是否存在卡尾未分配空间。
- 将新增的分区。
- 需要用户输入明确确认。

如果 TF 卡尾部已有未分配空间，应优先直接使用，不做缩小。如果没有未分配空间，脚本应停止并给出替代建议：使用 USB 临时 Linux、initramfs/rescue、或重新烧录时预留空间。

### Windows + 开发容器网络映射

Windows Docker/WSL 环境下，TF 卡读卡器不可用时，方案不再要求 WSL 或容器直接读取 TF 卡块设备。准备阶段由板上 Armbian minimal 修改自己的 TF 卡，开发机只负责构建、串口控制和文件传输。

网络路径应按以下优先级处理：

- 构建和 Starry quick-start 在 dev container 内执行。
- 串口由 dev container 访问，TF 卡块设备不一定由 dev container 访问。
- 板子固定 `192.168.100.101/24`，先验证它能 ping 通 Windows `192.168.100.100`。
- WSL 或容器如果能主动 SSH/SCP 到 `192.168.100.101`，就用主动推送。
- 如果容器无法直接访问板子，就由 WSL/Windows 中转，或用 Windows `portproxy` 转发 TCP 服务。

### U-Boot TFTP 不稳定

RK3588 Orange Pi 5 Plus 的 RTL8125 在 U-Boot 中未必可用。第一阶段必须以 `loady` 为默认加载路径，TFTP 只能作为可选优化。

### StarryOS 网络不稳定

在线上传首选网络，但第一阶段不能依赖网络。需要保留串口上传小包的路径。大型资产可先通过 Linux 中转写入 `starry-rootfs` 或 `testdata` 分区。

### Rootfs 默认选择误挂 Linux

必须显式设置：

```text
root=PARTLABEL=starry-rootfs
```

不要依赖默认 fallback，也不要依赖 `root=UUID=...`。

### SMP 风险

当前 RK3588 真机路径应保持 `max_cpu_num = 1`，直到 SMP + stack guard TLB shootdown 问题修复并验证。

## 推荐落地顺序

### 第一阶段：手工可验证最小闭环

1. 准备 `rootfs-aarch64-alpine.img`。
2. 串口登录板上 Armbian minimal，切到 `sudo -i`。
3. 配置板子网口为 `192.168.100.101/24`，确认能 ping 通 Windows `192.168.100.100`。
4. 把 rootfs 镜像传到板上 Linux。
5. 在板上检查 TF 卡布局，如果尾部有空闲空间，则创建 `starry-rootfs` ext4 分区。
6. 写入 rootfs，并确认 `PARTLABEL=starry-rootfs`。
7. quick-start `loady` 启动 StarryOS，并显式传 `root=PARTLABEL=starry-rootfs`。
8. 通过串口确认 StarryOS 挂载的是 `starry-rootfs`。
9. 手工通过串口上传一个小脚本并执行。

### 第二阶段：脚本化 TF 卡准备

实现 `prepare` 和 `reset` 两个脚本，支持 dry-run、布局报告和确认。

### 第三阶段：集成 quick-start root 参数

让 `cargo xtask starry quick-start orangepi-5-plus run` 能显式指定 rootfs 分区。

### 第四阶段：板测资产在线上传

实现 case 资产收集、串口上传、runner 执行、日志判定。

### 第五阶段：网络上传优化

在 StarryOS RK3588 网络路径稳定后，增加网络上传后端，降低串口传大文件的成本。

## 最终形态

理想日常命令流如下：

```bash
# 一次性或偶尔执行
cargo xtask starry rootfs --arch aarch64

# rootfs 镜像传到板上 Armbian 后，在板上 root shell 中执行
rk3588-board-prepare-starry-rootfs.sh \
  --device /dev/mmcblk0 \
  --rootfs /root/rootfs-aarch64-alpine.img \
  --size 4G

# 日常内核调试
cargo xtask starry quick-start orangepi-5-plus run \
  --serial /dev/ttyUSB0 \
  --root PARTLABEL=starry-rootfs

# 日常用户态 case 调试
cargo xtask starry board-case run \
  --board orangepi-5-plus \
  --case <case> \
  --serial /dev/ttyUSB0
```

这条链路把“慢而危险的 TF 卡分区准备”限制在一次性阶段，把“高频调试”收敛到 `loady` 启动内核和在线上传用户态资产。它也保留了原 Linux 镜像作为维护入口，避免 StarryOS 调试失败后必须重新烧卡救援。
