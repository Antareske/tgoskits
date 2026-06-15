#!/usr/bin/env bash
# build-image.sh - 从 assets 全量合成 RK3588 StarryOS 整盘镜像
#
# 用 proj57/rk3588/image-build 下的资产组装一张可直接烧录的整盘镜像：
#   idbloader.img + u-boot.itb (启动链)
#   starry-image.fit + starry.dtb + boot.scr (FAT boot 分区 p1)
#   starry-rootfs.ext4 (ext4 starry-rootfs 分区 p2)
#
# 用法:
#   ./build-image.sh [输出镜像路径]
# 环境变量:
#   ROOTFS   覆盖 rootfs 镜像 (默认 starry-rootfs.ext4)
#   FIT      覆盖 FIT 镜像 (默认 starry-image.fit)
#   DTB      覆盖 FAT 内 dtb (默认 starry.dtb)
#   SIZE_MB  镜像总大小 MiB (默认按 rootfs 自动计算)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

OUT="${1:-$HERE/output/rk3588-starryos-smp8.img}"
: "${ROOTFS:=starry-rootfs.ext4}"
: "${FIT:=starry-image.fit}"
: "${DTB:=starry.dtb}"
: "${SCR:=boot.scr}"

for f in idbloader.img u-boot.itb "$FIT" "$DTB" "$SCR" "$ROOTFS"; do
  [ -f "$f" ] || { echo "缺少资产 $f" >&2; exit 1; }
done

# 分区布局 (扇区, 512B)
P1_START=32768       # FAT boot @ 16 MiB
P2_START=262144      # ext4 rootfs @ 128 MiB
ROOTFS_BYTES=$(stat -c%s "$ROOTFS")
ROOTFS_SECTORS=$(( (ROOTFS_BYTES + 511) / 512 ))
P2_END=$(( P2_START + ROOTFS_SECTORS - 1 ))

if [ -n "${SIZE_MB:-}" ]; then
  TOTAL_BYTES=$(( SIZE_MB * 1024 * 1024 ))
else
  # rootfs 末尾再留 34 扇区给备份 GPT, 向上取整到 MiB
  TOTAL_SECTORS=$(( P2_END + 34 ))
  TOTAL_BYTES=$(( ((TOTAL_SECTORS * 512 + 1048575) / 1048576) * 1048576 ))
fi

echo ">> 输出: $OUT  (总 $((TOTAL_BYTES/1048576)) MiB, rootfs $((ROOTFS_BYTES/1048576)) MiB)"

rm -f "$OUT"
truncate -s "$TOTAL_BYTES" "$OUT"

# GPT
sgdisk -og "$OUT" >/dev/null
sgdisk -n 1:${P1_START}:$((P2_START-1)) -t 1:0700 -c 1:boot          "$OUT" >/dev/null
sgdisk -n 2:${P2_START}:${P2_END}       -t 2:8300 -c 2:starry-rootfs "$OUT" >/dev/null

# 启动链
dd if=idbloader.img of="$OUT" bs=512 seek=64    conv=notrunc status=none
dd if=u-boot.itb    of="$OUT" bs=512 seek=16384 conv=notrunc status=none

# FAT boot 分区 (112 MiB)
FAT_OFF=$(( P1_START * 512 ))
mformat -i "$OUT@@$FAT_OFF" -F -v BOOT ::
mcopy -i "$OUT@@$FAT_OFF" "$FIT" ::starry-image.fit
mcopy -i "$OUT@@$FAT_OFF" "$DTB" ::starry.dtb
mcopy -i "$OUT@@$FAT_OFF" "$SCR" ::boot.scr

# ext4 rootfs 分区
dd if="$ROOTFS" of="$OUT" bs=512 seek=${P2_START} conv=notrunc status=none

echo ">> 完成. 校验:"
sgdisk -p "$OUT" | sed -n '/Number/,$p'
echo "  idbloader: $(dd if="$OUT" bs=512 skip=64 count=1 status=none | od -An -tx1 | head -1 | tr -s ' ' | cut -c1-12)  (应 52 4b 4e 53)"
echo "  u-boot.itb: $(dd if="$OUT" bs=512 skip=16384 count=1 status=none | od -An -tx1 | head -1 | tr -s ' ' | cut -c1-12)  (应 d0 0d fe ed)"
