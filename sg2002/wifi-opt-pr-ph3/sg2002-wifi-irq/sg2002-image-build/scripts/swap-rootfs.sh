#!/usr/bin/env bash
# swap-rootfs.sh - 只换根文件系统, 直接对已有整盘镜像动刀
#
# 把新的 ext4 rootfs 写入镜像的 p2。启动链/FAT boot/内核都不动。
#   - 新 rootfs <= 现有 p2 容量: 原地 dd 写入。
#   - 新 rootfs >  现有 p2 容量: 自动增大镜像并扩大 p2, 然后写入。
# 本脚本就地修改传入的镜像; 如需保留原镜像, 由调用方先复制
# (编排入口 sg2002-image-build.sh update-rootfs 默认生成新镜像)。
#
# 用法:
#   ./swap-rootfs.sh <镜像路径> [新rootfs.ext4]
# 环境变量:
#   ASSETS_DIR 资产目录 (默认脚本所在目录)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
: "${ASSETS_DIR:=$HERE}"

IMG="${1:?用法: swap-rootfs.sh <镜像> [新rootfs.ext4]}"
NEW_ROOTFS="${2:-}"

[ -f "$IMG" ] || { echo "镜像不存在: $IMG" >&2; exit 1; }

# 若指定了新 rootfs, 更新资产副本
[ -n "$NEW_ROOTFS" ] && cp "$NEW_ROOTFS" "$ASSETS_DIR/rootfs.ext4"
ROOTFS="$ASSETS_DIR/rootfs.ext4"
[ -f "$ROOTFS" ] || { echo "缺少 $ROOTFS" >&2; exit 1; }

# 从 MBR 分区表解析 p2 的起始扇区和大小
P2_LINE=$(sfdisk -d "$IMG" 2>/dev/null | grep -v 'label:' | grep 'type=83' | head -1)
if [ -z "$P2_LINE" ]; then
  echo "错误: 未找到 type=83 (Linux) 分区" >&2
  sfdisk -l "$IMG" >&2
  exit 1
fi

P2_START=$(echo "$P2_LINE" | grep -oP 'start=\s*\K\d+')
P2_SIZE=$(echo "$P2_LINE" | grep -oP 'size=\s*\K\d+')
P2_CAP=$(( P2_SIZE * 512 ))
RF_BYTES=$(stat -c%s "$ROOTFS")

echo ">> p2 @ sector $P2_START (容量 $((P2_CAP/1048576)) MiB), 新 rootfs $((RF_BYTES/1048576)) MiB"

if [ "$RF_BYTES" -le "$P2_CAP" ]; then
  echo ">> 原地写入 (容量足够)"
  dd if="$ROOTFS" of="$IMG" bs=512 seek="$P2_START" conv=notrunc status=none
else
  echo ">> rootfs 超出现有 p2, 增大镜像并扩展分区"

  NEW_P2_SIZE=$(( (RF_BYTES + 511) / 512 ))
  NEW_P2_END=$(( P2_START + NEW_P2_SIZE - 1 ))
  NEW_TOTAL_SECTORS=$(( NEW_P2_END + 2048 )) # 末尾留 1 MB 余量
  NEW_TOTAL_BYTES=$(( ((NEW_TOTAL_SECTORS * 512 + 1048575) / 1048576) * 1048576 ))

  # 扩展镜像文件
  truncate -s "$NEW_TOTAL_BYTES" "$IMG"

  # 用 sfdisk 删除并重建 p2 (保留 p1)
  sfdisk --delete "$IMG" 2
  printf 'start=%d, size=%d, type=83\n' "$P2_START" "$NEW_P2_SIZE" | sfdisk -a "$IMG"

  dd if="$ROOTFS" of="$IMG" bs=512 seek="$P2_START" conv=notrunc status=none
fi

echo ">> rootfs 已换. 校验:"
PART_OFF=$((P2_START * 512))
if debugfs -R "stat /bin/sh" "${IMG}?offset=${PART_OFF}" 2>/dev/null | grep -q "Inode:"; then
  echo "  [ok] /bin/sh 存在"
else
  echo "  [警告] 无法读取新镜像中的 /bin/sh" >&2
fi
