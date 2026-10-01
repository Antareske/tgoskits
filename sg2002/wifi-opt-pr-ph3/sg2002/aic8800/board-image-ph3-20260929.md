# ph3 分支板测镜像构建记录（2026-09-29）

本文件记录 `sg2002/wifi-opt-pr-ph3` 分支第一张板测镜像的构建口径、处置与校验结果。

## 1. 产物

| 项 | 值 |
| --- | --- |
| 镜像 | `C:\Users\Asta\Desktop\build\sg2002_starryos_wifi_sta_ph3_20260929.img` |
| 大小 | 2578448384 字节（与基座一致） |
| SHA-256 | `6cbf1840a06ac5f210dc153bb67dfea8b1071f236acd44c4b0e350cf91154cfc` |
| 基座 | `sg2002_starryos_wifi_sta_ahead0_20260929.img`（只借其 p2 rootfs 与分区结构） |
| 内核 | `target/riscv64gc-unknown-none-elf/release/starryos.bin`，SHA-256 `b2406a93f5def4aae20f4ba5035afc1080118f3d49100d78e3b7da7b106786cf` |
| DTB | 派生件 `.sg2002-build/assets/licheerv-nano-sg2002-noinitrd.dtb`，SHA-256 `250ea0d9c72ba621afe6f5645d217d7793bff215dc86d9724f87318a60b503f8` |
| uimg | `starryos.uimg`，SHA-256 `9436af307395f2300a817f9ba0e1c0e28140eb26e3b57c9a6ec9ee1d2d877c06` |
| 来源提交 | 构建时的分支 tip；其后只改写了提交信息与 `drivers/net/aic8800/README.md` 一句说明，内核与 DTB 未变（哈希同上） |

## 2. 构建口径（可复现）

```bash
# 1) 板级内核（凭据编译期进驱动；log 取 Error）
AIC8800_FIRMWARE_DIR=<固件缓存> \
STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 \
  cargo xtask starry build -c www/sg2002/wifi-sta/licheerv-nano-sg2002-wifi-error.toml

# 2) 组装镜像（派生 DTB、重打 FIT、注入 p2 初始化文件、逐项回读校验）
BASE_IMAGE=<基座镜像.img> OUT_IMAGE=<输出.img> \
IMAGE_TOOL=<sg2002-image-build 目录> \
DEV_ROOTFS=<dev rootfs 镜像> \
STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 SKIP_KERNEL=1 \
  sh www/sg2002/wifi-sta/build-board-image.sh
```

- 板级配置是 `os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml` 的派生件，
  只把 `log = "Info"` 改成 `log = "Error"`，其余（features、target）逐字一致；
  它与同名 `.its` 一起放在 `www/sg2002/wifi-sta/`，构建时由 `-c` 指定，
  被项目追踪的那份配置没有改动。
- 同名 `.its`（只含 kernel 的 FIT 模板）是 `starryos.uimg` 的来源：axbuild 在
  配置同目录下找 `<配置名>.its`，缺了就不产出 uimg，`/starryos.uimg` 手动引导回退
  会停在基座镜像的旧内核上。
- 镜像输出目录按既有约定放 `C:\Users\Asta\Desktop\build`；`.sg2002-build/` 只放
  中间产物，已加入本工作树的本地 exclude，不进版本控制。

## 3. 新基线 dev 引入的三处板级回归与处置

本分支基线是 dev `5fd1c6c84`，其中包含 `fb3edd5cf`（`feat(initramfs): unify host image
boot flow`，#2528）。该提交给 SG2002 板级镜像带来三处回归，逐条如下：

1. **DTB 声明的 initrd 区间被当成 host initramfs**。`os/StarryOS/configs/board/licheerv-nano-sg2002.dtb`
   的 `/chosen` 写死了 `linux,initrd-start = <0x87e81000>` 与 `linux,initrd-end = <0x880ea1a0>`
   （官方 Linux 流程里该地址恒定放 ramdisk）。该提交之后内核把这段区间当 host initramfs
   严格校验，解不开就 panic。派生 DTB 用 `fdtput -d` 删掉这两个属性后，
   `someboot::initramfs_from_fdt` 返回 `None`，内核改走 SD 卡 rootfs。
2. **FIT 不能带 ramdisk**，否则那段地址放的就是厂商 ramdisk，与上一条撞同一个校验。
   派生件用 `www/sg2002/wifi-sta/boot-noinitrd.its` 重打 FIT。
3. **控制台拿不到 shell**。该提交同时删掉了 `legacy-board-init`，内核改为执行 rootfs 的
   `/sbin/init`；而基座镜像的 p2 是原版 Alpine inittab，`tty1..tty6` 六条
   `respawn:/sbin/getty` 在 StarryOS 下（没有虚拟控制台）无限重启。处置是把 dev rootfs 的
   `/etc/inittab`、`/usr/libexec/starry/console`、`/etc/profile.d/starry.sh` 注入 p2。

另有一件不属回归但同样必须的：**32 字节 `/chosen/rng-seed`**。WPA2 握手要启动熵，
`someboot` 要求该属性恰好 32 字节，而主线那份 DTB 没有这一项；上游的熵注入只接在
`starry run` 板卡路径上，裸构建 + 烧写这条路要自己补。派生件每次取新的随机数写入。

STA 凭据（SSID `aasta`、口令 `12345678`）由 `drivers/ax-driver/build.rs` 读环境变量、
经 `option_env!` 编入驱动，关联由驱动自己发起，rootfs 里不需要 wpa_supplicant。
缺凭据的症状是内核照常起、`wlan0` 也 UP 但永不关联，所以出镜像前用
`strings starryos.bin | grep -aasta` 自检。

## 4. 校验结果

脚本自检与独立复核（从镜像 FAT 取出 `boot.sd`、再从中取出内核与 DTB 逐字节比对）：

| 检查 | 结果 |
| --- | --- |
| FIT 默认配置 / Load 地址 | `config-sg2002_licheervnano_sd` / `0x80200000` |
| FIT 内是否有 ramdisk | 无 |
| FIT 内内核 == `target/.../starryos.bin` | 逐字节一致 |
| FIT 内 DTB == 派生件 | 逐字节一致 |
| `/chosen` 的 initrd 属性 | 0 处 |
| `/chosen` 的 rng-seed | 8 个字（32 字节） |
| `bootargs` | `root=/dev/mmcblk0p2 rootwait rw console=ttyS0,115200 ... loglevel=0` |
| DTB 的 aic 聚合属性 | `aic,tx-aggregation = <0x20>`、`aic,tx-aggregate-bytes = <0xc000>` |
| 内核含 SSID | `aasta` |
| p2 三个初始化文件 vs dev rootfs | 逐字节一致 |
| p2 `/starryos.uimg` | 与新构建的 uimg 一致 |
| p2 `/usr/bin/iperf3`、`/usr/lib/libiperf.so.0` | 在位（与 opt 线镜像中的两份逐字节一致） |
| p2 `/usr/bin/netmon` | 不在（本分支不含 eBPF 监测负载） |
| p2 `/bin/sh` | 可读 |

## 5. 与 opt 线的差异

- 内核来自本分支（前三阶段优化的整理分支，含本轮 OCR 修复的两笔提交），不含 opt 迭代分支上的
  探针与实验开关；DTB 因此也不含 `aic,tx-prepare-ahead` 等实验属性。
- rootfs 沿用 opt 线镜像的同一条 p2 血统（iperf3 等保持一致），只补了上面第 3 条的三个初始化文件。
- 日志级别为 `Error`，串口上看不到 `[wifi]` 的 info 级输出。

## 6. 烧写与回退

```bash
sudo dd if=<镜像>.img of=/dev/mmcblk0 bs=4M status=progress conv=fsync
```

串口 115200 8N1；autoboot 自动加载 p1 的 `boot.sd`。手动引导回退：

```
ext4load mmc 0:2 0x82200000 /starryos.uimg
bootm 0x82200000 - 0x81000000
```
