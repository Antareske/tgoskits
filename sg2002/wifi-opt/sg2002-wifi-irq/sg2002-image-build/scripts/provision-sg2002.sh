#!/usr/bin/env bash
# provision-sg2002.sh - 从官方 SG2002 Linux 镜像一次性提取 fip.bin / ramdisk.bin
#
# fip.bin (Cvitek 一级 bootloader) 与 ramdisk.bin (Cvitek 平台固化 ramdisk)
# 不可自行构建, 只能从官方 Linux 整盘镜像提取。提取后作为固定资产复用,
# 无需重复执行。
#
# 自动探测镜像内 FAT boot 分区的字节偏移 (优先按分区表解析 FAT 类型分区,
# 失败时尝试常见候选偏移), 不硬编码官方镜像布局。
#
# 用法:
#   ./provision-sg2002.sh <官方镜像.img> [assets_dir]
set -euo pipefail

IMG="${1:?用法: provision-sg2002.sh <官方镜像.img> [assets_dir]}"
AD="${2:-assets}"
[ -f "$IMG" ] || { echo "官方镜像不存在: $IMG" >&2; exit 1; }

mkdir -p "$AD"

# 候选偏移: 分区表中 FAT 类型 (c/0c/0b/01/0e) 的分区起始, 加上常见固定偏移
CANDS=()
while read -r start type; do
  case "$(echo "$type" | tr 'A-Z' 'a-z')" in
    c|0c|0b|01|0e) CANDS+=("$((start * 512))") ;;
  esac
done < <(sfdisk -d "$IMG" 2>/dev/null | awk -F'[,= ]+' '/^start=/{print $2, $NF}')
for off in 512 1048576 4194304 8388608; do
  CANDS+=("$off")
done

BOOT_OFF=""
for off in "${CANDS[@]}"; do
  # mdir 按 8.3 短名显示 (fip      bin), 不能依赖点号
  if mdir -i "${IMG}@@${off}" :: 2>/dev/null | grep -q "fip"; then
    BOOT_OFF="$off"
    break
  fi
done
[ -n "$BOOT_OFF" ] || { echo "未能在 $IMG 中定位含 fip.bin 的 FAT boot 分区" >&2; exit 1; }
echo ">> FAT boot 分区偏移: $BOOT_OFF ($((BOOT_OFF/512)) 扇区)"

mcopy -i "${IMG}@@${BOOT_OFF}" ::fip.bin "$AD/fip.bin"
mcopy -i "${IMG}@@${BOOT_OFF}" ::boot.sd "$AD/boot.sd"
dumpimage -T flat_dt -p 1 -o "$AD/ramdisk.bin" "$AD/boot.sd"
rm -f "$AD/boot.sd"

echo ">> 已提取:"
md5sum "$AD/fip.bin" "$AD/ramdisk.bin"
FIP_BYTES=$(stat -c%s "$AD/fip.bin")
if [ "$FIP_BYTES" -ne 440832 ]; then
  echo "  [警告] fip.bin 大小 $FIP_BYTES 与预期 440832 不一致, 请确认镜像来源" >&2
fi
