#!/usr/bin/env bash
# swap-rootfs.sh - 只换根文件系统(含 overlay), 直接对已有整盘镜像动刀
#
# 把新的 ext4 rootfs 写入镜像的 p2 (starry-rootfs)。启动链/FAT boot/内核都不动。
#   - 新 rootfs <= 现有 p2 容量: 原地 dd 写入。
#   - 新 rootfs >  现有 p2 容量: 自动增大镜像并把 p2 扩到能容纳, 重写 GPT 后写入。
#
# 用法:
#   ./swap-rootfs.sh <镜像路径> [新rootfs.ext4]
# 例 (用 tgoskits 下载的 rootfs):
#   ./swap-rootfs.sh ../rk3588-starryos-smp8.img \
#       /tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

IMG="${1:?用法: swap-rootfs.sh <镜像> [新rootfs.ext4]}"
NEW_ROOTFS="${2:-}"

[ -f "$IMG" ] || { echo "镜像不存在: $IMG" >&2; exit 1; }

# 若指定了新 rootfs, 更新 assets 副本
[ -n "$NEW_ROOTFS" ] && cp "$NEW_ROOTFS" starry-rootfs.ext4
ROOTFS=starry-rootfs.ext4
[ -f "$ROOTFS" ] || { echo "缺少 $ROOTFS" >&2; exit 1; }

P2_START=$(sgdisk -i 2 "$IMG" | awk '/First sector/{print $3}')
P2_END=$(sgdisk -i 2 "$IMG"   | awk '/Last sector/{print $3}')
P2_CAP=$(( (P2_END - P2_START + 1) * 512 ))
RF_BYTES=$(stat -c%s "$ROOTFS")
RF_SECTORS=$(( (RF_BYTES + 511) / 512 ))

echo ">> p2 @ sector $P2_START..$P2_END (容量 $((P2_CAP/1048576)) MiB), 新 rootfs $((RF_BYTES/1048576)) MiB"

if [ "$RF_BYTES" -le "$P2_CAP" ]; then
  echo ">> 原地写入 (容量足够)"
  dd if="$ROOTFS" of="$IMG" bs=512 seek="$P2_START" conv=notrunc status=none
else
  echo ">> rootfs 超出现有 p2, 增大镜像并重建 p2"
  NEW_P2_END=$(( P2_START + RF_SECTORS - 1 ))
  NEW_TOTAL_SECTORS=$(( NEW_P2_END + 34 ))
  NEW_TOTAL_BYTES=$(( ((NEW_TOTAL_SECTORS * 512 + 1048575) / 1048576) * 1048576 ))
  truncate -s "$NEW_TOTAL_BYTES" "$IMG"
  # 重定位备份 GPT 到新末尾, 删除并按新大小重建 p2 (保留 p1)
  sgdisk -e "$IMG" >/dev/null
  sgdisk -d 2 "$IMG" >/dev/null
  sgdisk -n 2:${P2_START}:${NEW_P2_END} -t 2:8300 -c 2:starry-rootfs "$IMG" >/dev/null
  dd if="$ROOTFS" of="$IMG" bs=512 seek="$P2_START" conv=notrunc status=none
fi

echo ">> rootfs 已换. 校验:"
sgdisk -i 2 "$IMG" | grep -E "Partition name|First sector|Last sector"
