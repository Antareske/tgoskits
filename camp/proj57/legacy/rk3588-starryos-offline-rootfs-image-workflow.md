# RK3588 StarryOS 离线 Rootfs 镜像制作工作流

## 目标

本文记录在 Windows + dev container 环境中，无法稳定通过 WSL 访问 TF 读卡器时，如何直接调整 Armbian raw 镜像，生成一份可一次性烧录到 TF 卡的 Orange Pi 5 Plus / RK3588 板测镜像。

目标布局对应 `rk3588-starryos-board-test-workflow-original.md` 中的 TF 卡 rootfs 规划：

- 保留可启动的 Armbian Linux 维护环境。
- 为 StarryOS 预置独立 rootfs 分区。
- StarryOS 分区使用 `PARTLABEL=starry-rootfs`，启动参数使用 `root=PARTLABEL=starry-rootfs`。
- 不依赖 TF 读卡器透传到 dev container。
- 镜像总大小适配常见标称 32GB SD/TF 卡。

最终推荐布局：

```text
raw image: 29.0 GiB

├── p1: rootfs          ext4  12.0 GiB  Armbian Linux
└── p2: starry-rootfs   ext4  17.0 GiB  StarryOS rootfs
```

选择 `29.0 GiB` 而不是 `32.0 GiB` 是为了适配标称 32GB 卡的实际可用容量。常见 32GB 卡实际容量约为 29.7GiB，29.0GiB raw 镜像有更高兼容性。

## 输入和输出

输入镜像：

```text
www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz
```

StarryOS rootfs 来源：

```text
https://github.com/rcore-os/tgosimages/releases/download/v0.0.5/rootfs-aarch64-alpine.img.tar.xz
```

输出镜像：

```text
www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_linux12g_starry17g_32sd.img.xz
```

## 工具依赖

在 Ubuntu/Debian dev container 中安装：

```bash
apt-get update
apt-get install -y fdisk parted gdisk
```

还需要系统已有或额外安装这些工具：

```text
xz
curl
tar
truncate
losetup
dd
e2fsck
resize2fs
tune2fs
mount
umount
```

## 标准流程

### 1. 解压原始 Armbian 镜像

```bash
xz -dk www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img.xz
```

得到：

```text
www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

### 2. 检查原始分区布局

```bash
parted -s www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img unit s print free
fdisk -l www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

本次使用的 Armbian 镜像原始布局只有一个 ext4 rootfs 分区：

```text
Disk: 3219456 sectors, about 1.54 GiB
p1: start 32768, end 3217407, name rootfs
```

### 3. 先扩展 Linux rootfs 到可维护大小

可以先把镜像扩到 16GiB 并扩展 p1/ext4，便于后续调整。该中间步骤不是最终布局要求，但在手工迭代时方便验证。

```bash
truncate -s 16G www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
sgdisk -e www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
parted -s www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img unit s resizepart 1 100%
```

扩展 ext4：

```bash
PART_LOOP=$(losetup --find --show \
  --offset $((32768*512)) \
  --sizelimit $(((33552384-32768+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

e2fsck -f -y "$PART_LOOP"
resize2fs "$PART_LOOP"
e2fsck -f -y "$PART_LOOP"
losetup -d "$PART_LOOP"
```

### 4. 禁用 Armbian 首次启动自动扩容

Armbian 镜像默认带有 `armbian-resize-filesystem.service`，首次启动会尝试扩展 rootfs。如果不禁用，它可能把整张 TF 卡剩余空间都分给 Linux rootfs，破坏 StarryOS 预留空间。

离线写入禁用标记：

```bash
mkdir -p /tmp/armbian-rootfs
mount -o rw,loop,offset=$((32768*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img \
  /tmp/armbian-rootfs

touch /tmp/armbian-rootfs/root/.no_rootfs_resize
rm -f /tmp/armbian-rootfs/root/.rootfs_resize
sync
umount /tmp/armbian-rootfs
```

不建议只写 `/root/.rootfs_resize` 限制大小，因为 Armbian 脚本在检测到剩余空间超过 1GiB 时可能自动创建额外分区，不符合保留整块未分配空间或手工规划 Starry 分区的目标。

### 5. 下载 StarryOS managed rootfs

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

本次 rootfs 原始大小为 1GiB。

### 6. 将 Linux ext4 缩小到 12GiB

先缩文件系统，再缩分区。顺序不能反。

当前 p1 仍从 sector `32768` 开始。12GiB ext4 对应：

```text
12 GiB / 512 B = 25165824 sectors
p1 start = 32768
p1 end = 32768 + 25165824 - 1 = 25198591
```

缩小 ext4：

```bash
PART_LOOP=$(losetup --find --show \
  --offset $((32768*512)) \
  --sizelimit $(((33552384-32768+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

e2fsck -f -y "$PART_LOOP"
resize2fs "$PART_LOOP" 12G
e2fsck -f -y "$PART_LOOP"
losetup -d "$PART_LOOP"
```

保留 p1 的 unique GUID 并用 `sgdisk` 重建 p1 到 12GiB：

```bash
sgdisk -i 1 www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

记录输出中的 `Partition unique GUID`，例如：

```text
AEC1B8C5-EE28-4605-BE91-A092135CC13D
```

重建 p1：

```bash
sgdisk \
  -d 1 \
  -n 1:32768:25198591 \
  -t 1:8305 \
  -c 1:rootfs \
  -u 1:AEC1B8C5-EE28-4605-BE91-A092135CC13D \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

### 7. 扩展 raw 镜像到 29GiB 并创建 starry-rootfs 分区

```bash
truncate -s 29G www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
sgdisk -e www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

创建 p2：

```bash
sgdisk \
  -n 2:25198592:0 \
  -t 2:8300 \
  -c 2:starry-rootfs \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

为了消除 GPT 备份表位置警告，可用 `gdisk` 的 expert command `k` 把 secondary partition table 移到镜像尾部：

```bash
printf 'x\nk\n\nw\ny\n' | gdisk \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

为保持 p2 尾部 2048 sector 对齐，可将 p2 尾部收回到 `60815359`。这一步不改变 ext4 实际可用容量，因为 ext4 按 4KiB block 对齐，之前多出的 1 个 512B sector 不会被文件系统使用。

```bash
P2_GUID=$(sgdisk -i 2 \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img \
  | awk -F': ' '/Partition unique GUID/ {print $2}')

sgdisk \
  -d 2 \
  -n 2:25198592:60815359 \
  -t 2:8300 \
  -c 2:starry-rootfs \
  -u 2:$P2_GUID \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
```

最终 sector 布局：

```text
p1 rootfs        start 32768     end 25198591  size 12.0 GiB
p2 starry-rootfs start 25198592  end 60815359  size 17.0 GiB
```

### 8. 写入并扩展 StarryOS rootfs

将 1GiB `rootfs-aarch64-alpine.img` 写入 p2：

```bash
P2_LOOP=$(losetup --find --show \
  --offset $((25198592*512)) \
  --sizelimit $(((60815359-25198592+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

dd if=tmp/axbuild/rootfs/rootfs-aarch64-alpine.img \
  of="$P2_LOOP" \
  bs=4M \
  conv=fsync \
  status=progress

e2fsck -f -y "$P2_LOOP"
resize2fs "$P2_LOOP"
e2fsck -f -y "$P2_LOOP"
losetup -d "$P2_LOOP"
```

设置 p2 的 ext4 filesystem label，便于 Linux 下识别。注意 StarryOS 启动依赖的是 GPT `PARTLABEL`，不是 ext4 label；这里设置 ext4 label 只是辅助。

```bash
P2_LOOP=$(losetup --find --show \
  --offset $((25198592*512)) \
  --sizelimit $(((60815359-25198592+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

tune2fs -L starry-rootfs "$P2_LOOP"
e2fsck -f -y "$P2_LOOP"
losetup -d "$P2_LOOP"
```

## 验证

### GPT 验证

```bash
sgdisk -v www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
sgdisk -p www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img
parted -s www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img unit GiB print free
```

预期重点：

```text
No problems found

1  32768     25198591  12.0 GiB  rootfs
2  25198592  60815359  17.0 GiB  starry-rootfs
```

`sgdisk -v` 可能提示 main partition table 和 first usable sector 之间有 2048 sector 对齐空洞。这是原始镜像和常见 util-linux/fdisk 对齐策略导致的非致命提示，可以保留。

### p1 文件系统验证

```bash
P1_LOOP=$(losetup --find --show \
  --offset $((32768*512)) \
  --sizelimit $(((25198591-32768+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

e2fsck -f -n "$P1_LOOP"
tune2fs -l "$P1_LOOP" | grep -E 'Filesystem volume name|Block count|Block size|Free blocks'
losetup -d "$P1_LOOP"
```

预期：

```text
Filesystem volume name: armbi_root
Block count: 3145728
Block size: 4096
```

### p2 文件系统验证

```bash
P2_LOOP=$(losetup --find --show \
  --offset $((25198592*512)) \
  --sizelimit $(((60815359-25198592+1)*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img)

e2fsck -f -n "$P2_LOOP"
tune2fs -l "$P2_LOOP" | grep -E 'Filesystem volume name|Block count|Block size|Free blocks'
losetup -d "$P2_LOOP"
```

预期：

```text
Filesystem volume name: starry-rootfs
Block count: 4452096
Block size: 4096
```

### Armbian 自动扩容禁用标记验证

```bash
mount -o ro,loop,offset=$((32768*512)) \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img \
  /tmp/armbian-rootfs

test -f /tmp/armbian-rootfs/root/.no_rootfs_resize
ls -l /tmp/armbian-rootfs/root/.no_rootfs_resize
umount /tmp/armbian-rootfs
```

## 压缩发布镜像

```bash
xz -T0 -6 -k -c \
  www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal.img \
  > www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_linux12g_starry17g_32sd.img.xz

xz -t www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_linux12g_starry17g_32sd.img.xz
xz -l www/Armbian_26.5.1_Orangepi5-plus_resolute_current_6.18.33_minimal_linux12g_starry17g_32sd.img.xz
```

本次产物信息：

```text
raw image: 31,138,512,896 bytes, 29.0 GiB
xz image:  about 397.9 MiB
```

## 烧录和启动使用

将最终 `.img.xz` 用常规镜像烧录工具写入 32GB 或更大 TF 卡。

StarryOS 启动参数必须显式指定：

```text
root=PARTLABEL=starry-rootfs
```

不要依赖默认 rootfs fallback，也不要使用 `root=UUID=...`，因为当前 StarryOS root 选择规则不支持 `root=UUID=...`。

## 注意事项

- 这是离线 raw 镜像编辑流程，不需要 dev container 能看到 TF 读卡器。
- 缩小文件系统前必须先运行 `e2fsck -f`。
- 缩小顺序必须是先 `resize2fs` 缩 ext4，再缩 GPT 分区。
- 扩大顺序通常是先扩大 GPT 分区，再 `resize2fs` 扩 ext4。
- 如果更换 Armbian 镜像版本，必须重新读取 p1 起止 sector、PARTUUID 和 rootfs 扩容脚本行为，不能盲目复用本文的固定 sector。
- 如果目标卡需要兼容更小容量，可以继续降低 p2 大小；如果目标卡为 64GB 或更大，可保持 29GiB 镜像，在烧录后用剩余未分配空间另建 testdata 分区，或离线生成更大的 p2。
