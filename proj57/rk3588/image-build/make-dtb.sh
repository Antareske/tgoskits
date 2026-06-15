#!/usr/bin/env bash
# make-dtb.sh - 从 tgoskits 官方 DTB 生成 starry.dtb（带 StarryOS bootargs）
#
# 必须使用官方 DTB os/StarryOS/configs/board/orangepi-5-plus.dtb：其 NPU 节点为
# `rockchip,rk3588-rknpu`，与 StarryOS 的 RKNPU 驱动匹配；Armbian 镜像内的 DTB
# 为 `rockchip,rk3588-rknn-core` 三核布局，StarryOS 不识别，NPU 无法初始化。
#
# 处理：保留 /chosen/stdout-path，把 bootargs 替换为指向 starry-rootfs 的命令行，
#       删除 linux,initrd-start/end（StarryOS 不用 initrd）。
#
# 用法:
#   ./make-dtb.sh [官方dtb路径]
# 默认从工作区 os/StarryOS/configs/board/orangepi-5-plus.dtb 取。
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

# 默认官方 DTB：assets 在 proj57/rk3588/image-build，工作区根在上溯 3 层
WS_ROOT="$(cd "$HERE/../../.." && pwd)"
SRC="${1:-$WS_ROOT/os/StarryOS/configs/board/orangepi-5-plus.dtb}"
BOOTARGS="root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4"

[ -f "$SRC" ] || { echo "找不到官方 DTB: $SRC" >&2; exit 1; }

tmp_dts="$(mktemp /tmp/starry-dtb.XXXXXX.dts)"
dtc -I dtb -O dts "$SRC" -o "$tmp_dts" 2>/dev/null

# 确认是匹配 StarryOS 驱动的 DTB
if ! grep -q 'rockchip,rk3588-rknpu' "$tmp_dts"; then
  echo "警告: $SRC 不含 rockchip,rk3588-rknpu 节点，NPU 可能无法工作" >&2
fi

python3 - "$tmp_dts" "$BOOTARGS" <<'PY'
import re, sys
path, bootargs = sys.argv[1], sys.argv[2]
s = open(path).read()
# 定位 chosen { ... } 块
m = re.search(r'(\bchosen\s*\{)(.*?)(\})', s, re.S)
if not m:
    sys.exit("未找到 chosen 节点")
body = m.group(2)
# 删除 initrd 行
body = re.sub(r'\s*linux,initrd-(start|end)\s*=\s*<[^>]*>;', '', body)
# 替换或追加 bootargs
if re.search(r'\bbootargs\s*=', body):
    body = re.sub(r'\bbootargs\s*=\s*"[^"]*";', f'bootargs = "{bootargs}";', body)
else:
    body = body.rstrip() + f'\n\t\tbootargs = "{bootargs}";\n\t'
# 确保 stdout-path 存在（earlycon 已够用，这里与已验证镜像保持一致）
if not re.search(r'\bstdout-path\s*=', body):
    body = '\n\t\tstdout-path = "serial2:1500000n8";' + body
s = s[:m.start(2)] + body + s[m.end(2):]
open(path, 'w').write(s)
PY

dtc -I dts -O dtb "$tmp_dts" -o starry.dtb 2>/dev/null
rm -f "$tmp_dts"

echo "已生成 $HERE/starry.dtb"
echo "  NPU: $(dtc -I dtb -O dts starry.dtb 2>/dev/null | grep -m1 'rk3588-rknpu' | tr -d '\t')"
echo "  bootargs: $(dtc -I dtb -O dts starry.dtb 2>/dev/null | grep -m1 bootargs | tr -d '\t')"
