# Starry Orange Pi 5 Plus 板测过程记录

## 背景

本次工作的目标是梳理 `StarryOS` 在 Orange Pi 5 Plus 上的 quick-start / board 流程，确认当前板测对 rootfs 的依赖方式，并判断是否能直接复用 tgoskits 的 managed rootfs 与 overlay 机制。

## 已完成的过程

1. 先确认了 Orange Pi quick-start 的现状。
2. 解析了 `ostool` 的 U-Boot / FIT / board 客户端流程。
3. 解析了 `ostool-server` 的 session、TFTP 和文件上传接口。
4. 解析了 `StarryOS` 内核侧的根文件系统选择逻辑。
5. 对比了 `test-suit/starryos` 的 QEMU rootfs overlay 流程和 board 流程。
6. 做了板上启动实验，并记录了串口、SMP、rootfs 以及用户态 shell 的表现。
7. 将当前工作中不需要的 quick-start tmp 配置缓存清理掉。

## 过程记录

### 1. Orange Pi quick-start 的启动方式

- quick-start Orange Pi 路径通过 U-Boot 串口流程启动。
- 现有流程会生成 FIT image，但只包含 kernel 和可选 DTB。
- U-Boot runner 会读取 `ramdisk_addr_r`，但没有把 ramdisk/rootfs 接入 boot 流程。
- Orange Pi quick-start 的配置生成只负责 build 配置和 U-Boot 配置，没有 rootfs 字段。

### 2. board / ostool-server 的真实能力

- `ostool-server` 提供的是“按 session 上传任意文件到 TFTP 目录”的能力。
- 文件通过 `PUT /api/v1/sessions/{session_id}/files` 上传，服务端落到 `ostool/sessions/<session_id>/...`。
- 这些文件可以暴露成 `tftp://...` URL，让 U-Boot 或板端去拉取。
- 但当前 board runner 只实际上传 DTB 和 FIT，没有“上传 rootfs 镜像并让板子作为块设备挂载”的路径。
- `BoardRunConfig` 也没有 rootfs/image/overlay 之类的字段。

### 3. Starry 内核侧的 rootfs 行为

- Starry 当前从 block device 枚举分区并挂载 root。
- root 选择支持：`/dev/mmcblk*`、`/dev/sd*`、`PARTUUID=`、`PARTLABEL=`。
- 当前不支持 `root=UUID=...`。
- 当前也没有 initrd/memdisk rootfs 的接入点。
- 也就是说，Starry board 启动依赖的仍然是真实块设备上的 rootfs，而不是 U-Boot session 上传的文件。

### 4. QEMU rootfs overlay 的来源

- `test-suit/starryos` 的 rootfs+overlay 机制主要是 QEMU 路径。
- QEMU 侧会下载/准备 managed rootfs，然后为每个 case 复制并注入 overlay。
- pipeline case 会把注入后的 rootfs 缓存在：

```text
target/<target>/qemu-cases/<build_group>/<case>/cache/rootfs/
```

- 这个机制不等同于 board 流程。

### 5. 实验中的板上现象

- 已把 Orange Pi 5 Plus 配置临时改成单核验证，确认单核能绕过之前的 `SMP + IPI + stack-guard-page` panic。
- 后续把该配置恢复回原来的 `max_cpu_num = 8`。
- quick-start 相关 tmp 配置缓存已清理：
  - `tmp/axbuild/config/starryos/quick-start`
  - `tmp/axbuild/.starry.toml`
- 板上日志还确认了：
  - Starry 能识别板上 TF 卡 ext4 分区并挂载为 rootfs。
  - 但用户态 `exit` 会触发 init 退出并走 `system_off`。

## 关键发现

1. `ostool` 的 board 机制支持文件上传，但不支持把 rootfs 作为 block root 交给 Starry 挂载。
2. `ostool-server` 的 session file / TFTP 能承载任意文件，但当前只用于 DTB / FIT 等启动产物。
3. `StarryOS` 当前只认真实块设备 rootfs，不认“网络上传的 rootfs 镜像”。
4. `test-suit/starryos` 的 rootfs+overlay 是 QEMU 专属逻辑，不是 board 专属逻辑。
5. Orange Pi quick-start 若要真正支持 managed rootfs，需要新增一层 rootfs 准备或写盘流程，而不是单纯改配置。

## 当前状态

- `os/StarryOS/configs/board/orangepi-5-plus.toml` 已恢复为原始 SMP 配置。
- quick-start tmp 配置缓存已删除。
- 当前主要未提交改动只剩串口 console 脚本增强。

## 可行方案

### 最小可行

- 继续使用板上 TF 卡 / USB 存储上的 ext4 rootfs。
- 通过 `PARTLABEL=rootfs` 或 `PARTUUID=` 让 root 选择更稳定。

### 中期方案

- 给 Orange Pi quick-start 增加 rootfs 准备提示或写盘辅助。
- 让 managed rootfs 能被显式写入板上介质。

### 大改方案

- 扩展 board runner / ostool-server / Starry 内核，让 rootfs 可以通过 initrd、ramdisk 或其他 block 形式注入。

## 后续建议

1. 如果目标是“让现有板测更稳”，优先补 `root=PARTLABEL=` / `root=UUID=` 支持。
2. 如果目标是“让板测自动准备系统镜像”，优先做 quick-start 的 rootfs 写盘辅助。
3. 如果目标是“完全摆脱板上预装 rootfs”，再考虑 ramdisk / initrd 方案。

## 参考文件

- `scripts/axbuild/src/starry/rootfs.rs`
- `scripts/axbuild/src/starry/quick_start.rs`
- `scripts/axbuild/src/starry/test.rs`
- `www/ostool/ostool/src/boot/fit.rs`
- `www/ostool/ostool/src/run/uboot.rs`
- `www/ostool/ostool/src/board/config.rs`
- `www/ostool/ostool-server/src/api/router.rs`
- `www/ostool/ostool-server/src/tftp/files.rs`
- `os/arceos/modules/axruntime/src/block/root.rs`
- `os/arceos/modules/axruntime/src/block/mod.rs`
- `test-suit/starryos/GUIDE.md`
