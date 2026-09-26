#!/usr/bin/env bash
# build-image.sh - 从资产全量合成 SG2002 StarryOS SD 卡镜像
#
# 组装一张可直接烧录的整盘镜像:
#   fip.bin (Cvitek 一级 bootloader)
#   boot.sd (FIT image = kernel + ramdisk + DTB)
#   rootfs.ext4 (ext4 rootfs)
#
# 用法:
#   ./build-image.sh [输出镜像路径]
# 环境变量:
#   ASSETS_DIR 资产目录 (默认脚本所在目录)
#   FIP        覆盖 fip.bin (默认 fip.bin)
#   FIT        覆盖 FIT 镜像 (默认 boot.sd)
#   ROOTFS     覆盖 rootfs 镜像 (默认 rootfs.ext4)
#   SIZE_MB    镜像总大小 MiB (默认按固定布局 2116 MiB)
#              rootfs 超出 p2 固定容量时自动扩展镜像与 p2 (除非显式给了 SIZE_MB)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
: "${ASSETS_DIR:=$HERE}"
cd "$ASSETS_DIR"

OUT="${1:-sg2002_starryos.img}"
: "${FIP:=fip.bin}"
: "${FIT:=boot.sd}"
: "${ROOTFS:=rootfs.ext4}"

for f in "$FIP" "$FIT" "$ROOTFS"; do
  [ -f "$f" ] || { echo "缺少资产 $f" >&2; exit 1; }
done

# 分区布局 (扇区, 512B) — 与 SG2002 U-Boot / 现有镜像约定一致
BOOT_SECTOR=2048
BOOT_SIZE_SECTORS=131072       # 64 MB
ROOTFS_SECTOR=133120
ROOTFS_SIZE_SECTORS=4198400    # ~2 GB
IMG_SIZE_SECTORS=4333568

ROOTFS_BYTES=$(stat -c%s "$ROOTFS")

if [ -n "${SIZE_MB:-}" ]; then
  IMG_SIZE_SECTORS=$(( SIZE_MB * 1024 * 1024 / 512 ))
  if [ "$((ROOTFS_BYTES / 512))" -ge "$((IMG_SIZE_SECTORS - ROOTFS_SECTOR))" ]; then
    echo "rootfs ($ROOTFS_BYTES bytes) 超出 SIZE_MB=$SIZE_MB 的 p2 容量" >&2
    exit 1
  fi
else
  # rootfs 超过固定 p2 容量时自动扩展 p2 与镜像
  NEEDED=$(( (ROOTFS_BYTES + 511) / 512 ))
  if [ "$NEEDED" -gt "$ROOTFS_SIZE_SECTORS" ]; then
    ROOTFS_SIZE_SECTORS=$(( NEEDED + 2048 ))      # 分区内留 1 MB 余量
    IMG_SIZE_SECTORS=$(( ROOTFS_SECTOR + ROOTFS_SIZE_SECTORS + 2048 ))
    echo ">> rootfs 超出默认 p2, 扩展 p2 至 $((ROOTFS_SIZE_SECTORS*512/1048576)) MiB"
  fi
fi

echo ">> 输出: $OUT  (总 $((IMG_SIZE_SECTORS*512/1048576)) MiB, rootfs $((ROOTFS_BYTES/1048576)) MiB)"

mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
dd if=/dev/zero of="$OUT" bs=512 count=0 seek=${IMG_SIZE_SECTORS}

# MBR 分区表
printf 'label: dos\nunit: sectors\nstart=%d, size=%d, type=c, bootable\nstart=%d, size=%d, type=83\n' \
  "$BOOT_SECTOR" "$BOOT_SIZE_SECTORS" "$ROOTFS_SECTOR" "$ROOTFS_SIZE_SECTORS" | sfdisk "$OUT"

# FAT boot 分区 (64 MB)
mkfs.fat -F 32 -n BOOT --offset ${BOOT_SECTOR} "$OUT"
BOOT_OFF=$((BOOT_SECTOR * 512))
mcopy -i "${OUT}@@${BOOT_OFF}" "$FIP" ::fip.bin
mcopy -i "${OUT}@@${BOOT_OFF}" "$FIT" ::boot.sd

# ext4 rootfs 分区
dd if="$ROOTFS" of="$OUT" bs=512 seek=${ROOTFS_SECTOR} conv=notrunc status=none

echo ">> 完成. 校验:"
sfdisk -l "$OUT" 2>/dev/null | grep -E "start|Size" | head -3
echo "  boot 内容:"
mdir -i "${OUT}@@${BOOT_OFF}" ::
