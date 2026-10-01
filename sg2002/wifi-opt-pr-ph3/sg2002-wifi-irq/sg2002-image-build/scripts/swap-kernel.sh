#!/usr/bin/env bash
# swap-kernel.sh - 只换内核/DTB, 直接对已有整盘镜像动刀, 不重建分区表/rootfs
#
# 流程: 用资产目录下当前的 starryos.bin (+ramdisk.bin +DTB) 重打 boot.sd,
#       重建 FAT boot 分区 (fip.bin 从原镜像提取保留, 写入新 boot.sd),
#       再以整块 FAT 镜像 dd 写回 p1。rootfs (p2) 不触碰。
#       本脚本就地修改传入的镜像; 如需保留原镜像, 由调用方先复制
#       (编排入口 sg2002-image-build.sh update-kernel 默认生成新镜像)。
#
# 为什么不在整盘镜像上直接 mcopy:
#   mtools 对已含数据的整盘镜像直接写 FAT (img@@offset) 会把 p2 区域清零
#   (WSL2 ext4 实测, 2026-08-28; reflink 共享块或全量 build 恰好掩盖该问题)。
#   独立 FAT 镜像仅经 dd 写回 p1 区域, 不触碰 p2, 安全。
#
# 用法:
#   ./swap-kernel.sh <镜像路径> [新内核bin] [新dtb]
# 环境变量:
#   ASSETS_DIR 资产目录 (默认脚本所在目录)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
: "${ASSETS_DIR:=$HERE}"

IMG="${1:?用法: swap-kernel.sh <镜像> [新内核bin] [新dtb]}"
NEW_KERNEL="${2:-}"
NEW_DTB="${3:-}"

[ -f "$IMG" ] || { echo "镜像不存在: $IMG" >&2; exit 1; }
IMG="$(realpath "$IMG")"

# 若指定了新内核/新dtb, 先更新资产副本 (与目标相同则跳过, 避免 cp 自身报错)
if [ -n "$NEW_KERNEL" ]; then
  [ "$(realpath "$NEW_KERNEL")" != "$(realpath "$ASSETS_DIR/starryos.bin")" ] && \
    cp "$NEW_KERNEL" "$ASSETS_DIR/starryos.bin"
fi
if [ -n "$NEW_DTB" ]; then
  [ "$(realpath "$NEW_DTB")" != "$(realpath "$ASSETS_DIR/licheerv-nano-sg2002.dtb")" ] && \
    cp "$NEW_DTB" "$ASSETS_DIR/licheerv-nano-sg2002.dtb"
fi

# 重打 boot.sd
ASSETS_DIR="$ASSETS_DIR" "$HERE/repack-fit.sh"

cd "$ASSETS_DIR"
BOOT_SECTOR=2048
BOOT_OFF=$((BOOT_SECTOR * 512))

# 验证 boot.sd 不超过 boot 分区 (64 MB)
FIT_BYTES=$(stat -c%s boot.sd)
BOOT_CAP=$((131072 * 512))
if [ "$FIT_BYTES" -ge "$BOOT_CAP" ]; then
  echo "boot.sd ($FIT_BYTES bytes) 超出 boot 分区容量 ($BOOT_CAP bytes)." >&2
  exit 1
fi

# 重建 FAT boot 分区: fip.bin (从原镜像提取) + 新 boot.sd → 独立 FAT 镜像 → dd 写回
TMPD="$(mktemp -d)"
trap 'rm -rf "$TMPD"' EXIT
if ! mcopy -i "${IMG}@@${BOOT_OFF}" ::fip.bin "$TMPD/fip.bin" 2>/dev/null; then
  echo "错误: 无法从镜像 FAT 提取 fip.bin, 中止 (boot 分区结构异常)" >&2
  exit 1
fi
truncate -s "$BOOT_CAP" "$TMPD/boot-part.img"
mkfs.fat -F 32 -n BOOT "$TMPD/boot-part.img" >/dev/null
mcopy -o -i "$TMPD/boot-part.img" "$TMPD/fip.bin" ::fip.bin
mcopy -o -i "$TMPD/boot-part.img" boot.sd ::boot.sd
dd if="$TMPD/boot-part.img" of="$IMG" bs=512 seek="$BOOT_SECTOR" conv=notrunc status=none

echo ">> 内核已换. FAT 内容:"
mdir -i "${IMG}@@${BOOT_OFF}" ::
echo ">> p2 完整性:"
if debugfs -R "stat /bin/sh" "${IMG}?offset=$((133120 * 512))" 2>/dev/null | grep -q "^Inode:"; then
  echo "  [ok] /bin/sh"
else
  echo "  [警告] 无法读取镜像 p2 rootfs, 请检查" >&2
fi
