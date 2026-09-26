#!/usr/bin/env bash
# repack-fit.sh - 用内核 + DTB + ramdisk 重打 boot.sd (FIT image)
#
# boot.its 通过 /incbin/ 引用同目录下的 starryos.bin、ramdisk.bin 与 DTB，
# 直接 mkimage 打包即可。
#
# 用法:
#   ./repack-fit.sh
# 环境变量:
#   ASSETS_DIR 资产目录 (默认脚本所在目录)
#   KERNEL     内核二进制 (默认 starryos.bin, 相对 ASSETS_DIR 或绝对路径)
#   DTB        设备树 (默认 licheerv-nano-sg2002.dtb)
#   RAMDISK    ramdisk (默认 ramdisk.bin)
#   ITS        ITS 模板 (默认 ../templates/boot.its 或 ASSETS_DIR/boot.its)
#   FIT        输出 FIT 镜像 (默认 boot.sd)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILL_ROOT="$(cd "$HERE/.." && pwd)"
: "${ASSETS_DIR:=$HERE}"

if [ -z "${ITS:-}" ]; then
  if [ -f "$SKILL_ROOT/templates/boot.its" ]; then
    ITS="$SKILL_ROOT/templates/boot.its"
  else
    ITS="$ASSETS_DIR/boot.its"
  fi
fi

cd "$ASSETS_DIR"
: "${KERNEL:=starryos.bin}"
: "${DTB:=licheerv-nano-sg2002.dtb}"
: "${RAMDISK:=ramdisk.bin}"
: "${FIT:=boot.sd}"

for f in "$KERNEL" "$DTB" "$RAMDISK" "$ITS"; do
  [ -f "$f" ] || { echo "缺少 $f" >&2; exit 1; }
done

# mkimage 按 ITS 文件所在目录解析 /incbin/ 相对路径:
# ITS 不在资产目录时, 先复制一份到资产目录再打包
if [ "$(cd "$(dirname "$ITS")" && pwd)" = "$(pwd)" ]; then
  MKITS="$ITS"
else
  MKITS="$PWD/_boot.its.tmp"
  cp "$ITS" "$MKITS"
fi

mkimage -f "$MKITS" "$FIT"
[ "$MKITS" != "$ITS" ] && rm -f "$MKITS"
echo "已生成 $ASSETS_DIR/$FIT"
mkimage -l "$FIT" | grep -E "Load Address|Entry Point|Data Size" | head -6
