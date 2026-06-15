#!/usr/bin/env bash
# swap-kernel.sh - 只换内核/设备树, 直接对已有整盘镜像动刀, 不重建分区表/rootfs
#
# 流程: 用 assets 下当前的 starryos.bin (+starry.dtb) 重打 FIT, 然后把新的
#       starry-image.fit / starry.dtb 写回镜像的 FAT boot 分区 (p1)。
#
# 用法:
#   ./swap-kernel.sh <镜像路径> [新内核bin] [新dtb]
# 例:
#   ./swap-kernel.sh ../rk3588-starryos-smp8.img \
#       ../../../target/aarch64-unknown-none-softfloat/release/starryos.bin
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

IMG="${1:?用法: swap-kernel.sh <镜像> [新内核bin] [新dtb]}"
NEW_KERNEL="${2:-}"
NEW_DTB="${3:-}"

[ -f "$IMG" ] || { echo "镜像不存在: $IMG" >&2; exit 1; }

# 若指定了新内核/新dtb, 先更新 assets 副本
[ -n "$NEW_KERNEL" ] && cp "$NEW_KERNEL" starryos.bin
[ -n "$NEW_DTB" ]    && cp "$NEW_DTB"    starry.dtb

# 重打 FIT
./repack-fit.sh

# 找到 FAT boot 分区起始字节 (p1)
P1_START_SECTOR=$(sgdisk -i 1 "$IMG" | awk '/First sector/{print $3}')
FAT_OFF=$(( P1_START_SECTOR * 512 ))
echo ">> FAT boot @ sector $P1_START_SECTOR (offset $FAT_OFF)"

# FIT 体积检查 (p1 = 112 MiB)
P1_END_SECTOR=$(sgdisk -i 1 "$IMG" | awk '/Last sector/{print $3}')
P1_BYTES=$(( (P1_END_SECTOR - P1_START_SECTOR + 1) * 512 ))
FIT_BYTES=$(stat -c%s starry-image.fit)
if [ "$FIT_BYTES" -ge "$P1_BYTES" ]; then
  echo "FIT ($FIT_BYTES) 超出 boot 分区容量 ($P1_BYTES). 需增大 p1 或改用 build-image.sh 重建." >&2
  exit 1
fi

# 写回 FAT 内文件 (覆盖)
mcopy -o -i "$IMG@@$FAT_OFF" starry-image.fit ::starry-image.fit
mcopy -o -i "$IMG@@$FAT_OFF" starry.dtb       ::starry.dtb

echo ">> 内核已换. FAT 内容:"
mdir -i "$IMG@@$FAT_OFF" ::
