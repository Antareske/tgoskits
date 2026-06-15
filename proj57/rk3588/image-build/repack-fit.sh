#!/usr/bin/env bash
# repack-fit.sh - 用当前 starryos.bin + starry.dtb 重新打包 starry-image.fit
# 换新内核或换新设备树后调用。产物覆盖 assets/starry-image.fit。
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

: "${KERNEL:=starryos.bin}"
: "${DTB:=starry.dtb}"
: "${ITS:=starry.its}"
: "${FIT:=starry-image.fit}"

for f in "$KERNEL" "$DTB" "$ITS"; do
  [ -f "$f" ] || { echo "缺少 $f" >&2; exit 1; }
done

# starry.its 通过 /incbin/ 引用 starryos.bin 与 starry.dtb，直接打包即可
mkimage -f "$ITS" "$FIT"
echo "已生成 $HERE/$FIT"
mkimage -l "$FIT" | grep -E "Load Address|Entry Point|Data Size" | head -4
