---
name: sg2002-image-build
description: Build, update, and customize SG2002 (LicheeRV Nano) StarryOS SD card images from a tgoskits workspace. Use when the user wants to assemble a bootable SG2002 image, compile StarryOS for SG2002 from a chosen tgoskits source tree or commit, inject files/apps into the rootfs, swap kernel or rootfs in an existing image, or provision fip.bin/ramdisk.bin from an official image.
---

# SG2002 StarryOS Image Build

## 用途

从 tgoskits 源码构建 SG2002 (LicheeRV Nano) StarryOS SD 卡镜像，支持：

- 全量构建：编译 StarryOS 内核 → 准备 Alpine rootfs → 注入资产 → 组装整盘镜像；
- 自定义 rootfs：注入任意文件/应用（`--inject` / `--manifest` / `--init-assets`）；
- 指定编译来源：任意 tgoskits 工作树（`--source`）或某个提交版本（`--commit`），任意板级配置（`--config`）；
- 快速更新：只换内核或 rootfs，默认生成新镜像，`--overwrite` 就地覆盖已有镜像；
- 中间产物统一组织在 `<tgoskits根>/.sg2002-build/`（不在版本控制内）。

全部流程只依赖主线 tgoskits 的路径与命令，不引用任何分支特有内容。

## 前置依赖

### 主机工具

```bash
# Debian/Ubuntu (含 WSL2)
sudo apt install mtools u-boot-tools dosfstools e2fsprogs util-linux
```

`check-deps` 子命令可检查缺失项。编译 StarryOS 还需要主线 tgoskits 的 Rust 工具链（见主线构建文档）。

### 一次性资产

`fip.bin`（Cvitek 一级 bootloader）与 `ramdisk.bin`（Cvitek 平台固化 ramdisk）不可自行构建，需从官方 Linux 整盘镜像提取一次：

```bash
scripts/sg2002-image-build.sh provision <官方Linux镜像.img>
```

脚本自动探测镜像内 FAT boot 分区偏移（分区表 + 常见候选），提取后打印 md5 并校验 `fip.bin` 大小（预期 440832 字节）。官方镜像来源：Sipeed LicheeRV-Nano-Build release 的 LicheeRV Nano 整盘镜像。提取结果存入 `assets/`，之后复用，无需重复。

## 工作目录布局

```
<工作树根>/.sg2002-build/
├── assets/                  # 组装输入 (全部由脚本维护)
│   ├── fip.bin              #   provision 提取, 固定资产
│   ├── ramdisk.bin          #   provision 提取, 固定资产
│   ├── licheerv-nano-sg2002.dtb   # 主线 os/StarryOS/configs/board/ 副本
│   ├── starryos.bin         #   cargo xtask starry build 产物副本
│   ├── starryos.uimg        #   内核 uimg 副本 (手动引导回退用)
│   ├── rootfs.ext4          #   Alpine rootfs 工作副本 (注入在此进行)
│   └── boot.sd              #   repack-fit 输出 (FIT)
├── output/                  # 镜像产物 *.img + *.json
│   └── latest.img           #   指向最近产物的软链
├── sources/<rev>/           # --commit 的 detached worktree (git worktree add)
└── work/                    # 临时文件
```

`.sg2002-build/` 不入版本控制（不修改被项目追踪的 .gitignore；如需忽略，自行在本地未追踪的 .git/info/exclude 中添加）。

## 快速开始

```bash
S=scripts/sg2002-image-build.sh
cd <tgoskits 工作树根>

# 1) 一次性: 提取 bootloader 资产
$S provision <官方Linux镜像.img>

# 2) 全量构建 (编译内核 + 下载 rootfs + 组装)
$S build --inject app/act-infer:/root/act-infer:0755 \
         --inject models/act_det.cvimodel:/root/act_det.cvimodel \
         --init-assets
# 产物: .sg2002-build/output/sg2002_starryos_YYYYMMDD-HHMMSS.img

# 3) 迭代: 换内核 / 换 rootfs (默认新镜像, 源镜像不动)
$S update-kernel .sg2002-build/output/<镜像>.img
$S update-rootfs .sg2002-build/output/<镜像>.img
```

烧写（不在脚本内，板相关操作需自行执行）：

```bash
sudo dd if=<镜像>.img of=/dev/mmcblk0 bs=4M status=progress conv=fsync
```

串口 115200 8N1，autoboot 自动加载 p1 `boot.sd` 启动 StarryOS。

## 命令

### provision `<官方镜像.img>`

一次性提取 `fip.bin` / `ramdisk.bin` 到 `assets/`。见「一次性资产」。

### build

```
$S build [--source <path>] [--commit <rev>] [--config <toml>] [--dtb <file>]
         [--kernel <bin>] [--rootfs <ext4>]
         [--inject host:target[:mode]]... [--manifest <file>]... [--init-assets]
         [-o <out.img>]
```

步骤：编译内核 → 复制 DTB → 准备 rootfs → 注入资产 → 注入 `/starryos.uimg` → 打包 FIT → 组装镜像 → 写入 buildinfo + 校验。

| 选项 | 默认 | 说明 |
|------|------|------|
| `--source <path>` | 当前工作树根 | starry 编译来源，任意 tgoskits 树/工作树 |
| `--commit <rev>` | 无 | 在 `--source` 检出该提交编译（`git worktree add --detach` 到 `sources/<rev>/`，不动原树；需先 fetch） |
| `--config <toml>` | `os/StarryOS/configs/board/licheerv-nano-sg2002.toml` | 板级配置，相对 `--source` 或绝对路径 |
| `--dtb <file>` | config 同名 `.dtb`，回退主线 `licheerv-nano-sg2002.dtb` | 覆盖设备树 |
| `--kernel <bin>` | 现场编译 | 复用已有内核，跳过编译 |
| `--rootfs <ext4>` | 复用 `assets/rootfs.ext4`，否则复用已下载，否则 `cargo xtask starry rootfs --arch riscv64` 下载 | 复用已有 rootfs；重新下载请删除 `assets/rootfs.ext4` |
| `--inject` / `--manifest` / `--init-assets` | 无 | 见「Rootfs 定制」 |
| `-o <out.img>` | `output/sg2002_starryos_<时间戳>.img` | 输出镜像；指向已存在文件时覆盖 |

内核产物取自主线路径 `<source>/target/riscv64gc-unknown-none-elf/release/starryos.bin`（及同目录 `starryos.uimg`）；rootfs 下载产物为 `<source>/.tgos-images/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img`。

每个产物旁生成同名 `.json` buildinfo（来源、提交、配置、注入清单），并更新 `output/latest.img` 软链。

### update-kernel `<镜像.img>`

```
$S update-kernel <镜像.img> [--kernel <bin>] [--dtb <file>] [--overwrite | -o <out.img>] [--no-sync-uimg]
```

只替换 FAT boot 分区中的 `boot.sd`（内核 + DTB），分区表与 rootfs 不动。默认复制源镜像为新文件 `<名>_kernel-<时间戳>.img`；`--overwrite` 就地修改源镜像。内核默认用 `assets/starryos.bin`。

- 默认把 `assets/starryos.uimg` 同步注入镜像 rootfs（`/starryos.uimg`），保证手动引导回退与 boot.sd 内核一致；`--no-sync-uimg` 关闭；
- `--kernel` 同目录的 `.uimg` 存在时自动同步到资产，保持 `assets` 内 bin/uimg 配对；
- 完成后生成同名 `.json` buildinfo（operation=update-kernel）并更新 `latest.img` / `latest.json`。

### update-rootfs `<镜像.img>`

```
$S update-rootfs <镜像.img> [--rootfs <ext4>] [--overwrite | -o <out.img>]
```

只替换 p2 ext4 rootfs 分区，启动链不动。默认复制源镜像为新文件 `<名>_rootfs-<时间戳>.img`；`--overwrite` 就地修改。新 rootfs 超出 p2 容量时自动扩大镜像并扩展 p2。完成后同样生成 buildinfo 并更新 `latest.img` / `latest.json`。

### inject

```
$S inject [--inject ...] [--manifest ...] [--init-assets]
```

仅对 `assets/rootfs.ext4` 执行注入（`build` 的同名步骤，便于单独迭代 rootfs 内容）。底层 `inject-rootfs.sh` 也支持直接对整盘镜像的 rootfs 分区注入（传 `"<镜像.img>?offset=<分区字节偏移>"` 作为 rootfs 参数），用于 `update-kernel` 的 `/starryos.uimg` 同步等场景。

### check-deps / clean

- `check-deps`：检查主机工具是否齐全。
- `clean`：清理 `work/` 临时文件（保留 `assets/` 与 `output/`）。

## Rootfs 定制

注入通过 `debugfs` 进行（WSL2 无 loop 设备时的通用方式），引擎规则：

- 目标已存在 → 先删后写；缺失的父目录逐级自动创建；
- `mode` 为 4 位八进制（如 `0755`），通过 `set_inode_field` 设置；
- `--inject host:target[:mode]` 的 host 相对当前目录；`--manifest` 中 host 相对 manifest 文件所在目录；
- 所有注入幂等，可重复执行。

### manifest 文件

```
# 每行: <host 路径> <rootfs 内绝对路径> [mode]
app/act-infer /root/act-infer 0755
models/act_det.cvimodel /root/act_det.cvimodel
config/sshd_config /etc/ssh/sshd_config
```

示例见 `templates/manifest.example`。

### 内置 init 套件（`--init-assets`）

注入 `templates/init-assets/` 下的系统初始化资产：

| 文件 | 目标 | 说明 |
|------|------|------|
| `starry-init.sh` | `/usr/bin/starry-init.sh` (0755) | 启动 sshd、WiFi DHCP |
| `sshd_config` | `/etc/ssh/sshd_config` | OpenSSH 10.2+ 兼容（无 UsePAM/UsePrivilegeSeparation） |
| `inittab` | `/etc/inittab` | openrc 完成后 `::once:/usr/bin/starry-init.sh` |
| — | `/etc/profile` 末尾追加 | 安全网：inittab 失败时登录仍触发初始化（幂等） |
| — | `/var/empty` uid/gid → 0:0 | sshd 要求（存在才设置） |

sshd 可执行文件与 host key 需另行注入（如从 Alpine 3.23 riscv64 仓库提取）。

## 镜像布局与关键约束

```
SD 卡镜像 (MBR/DOS, 默认 2.1 GB)
├── Sector 0-2047:  MBR + 预留
├── Partition 1:    FAT32 64 MB (bootable, type 0x0C) — fip.bin + boot.sd
├── Partition 2:    ext4 ~2 GB (type 0x83) — Alpine rootfs + /starryos.uimg
```

| 参数 | 值 |
|------|-----|
| Boot 分区起始/大小 | 2048 / 131072 扇区 (64 MB) |
| Rootfs 分区起始/大小 | 133120 / 4198400 扇区（rootfs 超容时自动扩展） |
| 镜像总扇区 | 4333568 |
| StarryOS 加载地址 | `0x80200000` |

启动流程：`ROM → fip.bin (FSBL+BL31+U-Boot SPL) → boot.sd (FIT, autoboot) → StarryOS kernel → /bin/sh`。

### boot.its 硬编码字段（不可修改）

| 字段 | 值 |
|------|-----|
| `configurations/default` | `config-sg2002_licheervnano_sd`（SG2002 U-Boot 硬编码） |
| fdt 节点名 | `fdt-sg2002_licheervnano_sd` |
| kernel `load` / `entry` | `0x80200000` |
| hash 算法 | kernel/ramdisk `crc32`，fdt `sha256`（对齐官方 boot.sd） |

### 设备树

必须使用主线 `os/StarryOS/configs/board/licheerv-nano-sg2002.dtb`（~24 KB），不可用官方 Linux 镜像的 DTB（~22 KB，驱动不匹配会导致启动后无输出）。

## 手动引导回退

autoboot 异常时在 U-Boot 倒计时打断：

```
ext4load mmc 0:2 0x82200000 /starryos.uimg
bootm 0x82200000 - 0x81000000
```

`build` 自动把 `starryos.uimg` 注入 rootfs `/starryos.uimg`（uimg 加载地址 `0x82200000` 与内核解包目标 `0x80200000` 不重叠）。`update-kernel` 默认同样同步（见上）。

## 验证

```bash
# boot.sd 内容
mkimage -l .sg2002-build/assets/boot.sd
#   确认: Default Configuration = config-sg2002_licheervnano_sd, Load = 0x80200000

# 镜像 boot 分区
mdir -i "镜像.img@@$((2048*512))" ::

# 镜像 rootfs 分区
debugfs -R "stat /bin/sh" "镜像.img?offset=$((133120*512))"
```

`build` 结束时会自动执行上述校验并输出结果；`update-*` 同样执行（含从镜像 FAT 提取 `boot.sd` 后 `mkimage -l` 校验 Load 地址与默认配置名）。

## 常见问题

### U-Boot: Could not find configuration node

`boot.its` 的 `configurations/default` 必须为 `config-sg2002_licheervnano_sd`，本 skill 的模板已固定，不要修改。

### 启动后 StarryOS 无输出

确认 DTB 来自主线项目（~24 KB）而非官方 Linux 镜像（~22 KB）。

### swap 后镜像 rootfs (p2) 损坏 / 变空

**不要在含数据的整盘镜像上直接用 mtools 写 FAT**（`mcopy -o -i "镜像.img@@<偏移>"`）：实测会把 p2 区域清零（WSL2 ext4，2026-08-28；reflink 共享块或全量 build 流程恰好掩盖此问题）。`swap-kernel.sh` 已改为独立 FAT 镜像（mkfs.fat + mcopy）+ `dd` 写回 p1，脚本内部校验 p2 完整性。同样地，复制镜像基底时避免 `cp --reflink=auto`（WSL2 ext4 上 reflink 复制 2GB 镜像偶发内容异常），编排入口已用 `--reflink=never`。

### rootfs 需要强制重新下载

删除 `.sg2002-build/assets/rootfs.ext4`（以及 `--source` 树中 `.tgos-images/` 下对应文件），再执行 `build`。

### 使用带改动的内核

`build` 不做源码 overlay。如需带本地改动的内核：在 `--source` 树上应用改动并提交，再用 `--commit <rev>` 构建；或直接 `--source` 指向已带改动的树。

## 脚本组成（可直接调用）

| 脚本 | 职责 |
|------|------|
| `scripts/sg2002-image-build.sh` | 编排入口（本 SKILL 所有命令） |
| `scripts/provision-sg2002.sh` | 官方镜像 → fip.bin / ramdisk.bin |
| `scripts/repack-fit.sh` | starryos.bin + DTB + ramdisk → boot.sd |
| `scripts/build-image.sh` | fip + boot.sd + rootfs → 整盘镜像 |
| `scripts/swap-kernel.sh` | 就地换镜像内核（重建 FAT 写回 p1，不触碰 p2；调用方先复制则得到新镜像） |
| `scripts/swap-rootfs.sh` | 就地换镜像 rootfs（自动扩分区） |
| `scripts/inject-rootfs.sh` | debugfs 注入引擎（支持独立 rootfs.ext4 与镜像分区 `img?offset=`） |
| `templates/boot.its` | FIT 模板（硬编码字段勿改） |
| `templates/init-assets/` | 内置 init 套件 |
