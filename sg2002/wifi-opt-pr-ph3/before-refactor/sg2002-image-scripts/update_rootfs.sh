#!/usr/bin/env bash
#
# update_rootfs.sh — 替换 ext4 rootfs 并生成新 SD 卡镜像
#
# 基于已有镜像创建新镜像，替换其中的 rootfs ext4 分区。
# 原镜像不做任何修改，产出为独立的新镜像文件。
#
# 用法:
#   ./www/sg2002-image-scripts/update_rootfs.sh [SRC_IMAGE] [ROOTFS_IMG] [OUT_IMAGE]
#
#   SRC_IMAGE   源镜像路径（不会被修改），默认为 target/sg2002/sg2002_starryos_wifi.img
#   ROOTFS_IMG  新 rootfs 镜像路径，默认自动下载（cargo xtask starry rootfs --arch riscv64）
#   OUT_IMAGE   新镜像路径，默认在源镜像同目录生成带时间戳的文件名
#
# 需要从仓库根目录执行。
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

SRC_IMAGE="${1:-target/sg2002/sg2002_starryos_wifi.img}"
ROOTFS_SECTOR=133120

if [[ ! -f "${SRC_IMAGE}" ]]; then
    echo "错误: 源镜像不存在: ${SRC_IMAGE}"
    echo "用法: $0 [SRC_IMAGE] [ROOTFS_IMG] [OUT_IMAGE]"
    exit 1
fi

if [[ $# -ge 2 ]]; then
    ROOTFS_IMG="$2"
else
    echo "=== Step 1: 下载/准备 rootfs ==="
    cargo xtask starry rootfs --arch riscv64
    ROOTFS_IMG="tmp/axbuild/rootfs/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img"
fi

if [[ ! -f "${ROOTFS_IMG}" ]]; then
    echo "错误: rootfs 镜像未找到: ${ROOTFS_IMG}"
    echo "请先运行: cargo xtask starry rootfs --arch riscv64"
    echo "或指定路径: $0 [SRC_IMAGE] [ROOTFS_IMG] [OUT_IMAGE]"
    exit 1
fi

# 默认在源镜像同目录生成带时间戳的新文件名
if [[ $# -ge 3 ]]; then
    OUT_IMAGE="$3"
else
    SRC_DIR="$(dirname "${SRC_IMAGE}")"
    SRC_BASE="$(basename "${SRC_IMAGE}" .img)"
    TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
    OUT_IMAGE="${SRC_DIR}/${SRC_BASE}_rootfs-${TIMESTAMP}.img"
fi

if [[ -f "${OUT_IMAGE}" ]]; then
    echo "错误: 输出镜像已存在: ${OUT_IMAGE}"
    exit 1
fi

ROOTFS_SIZE="$(stat -c%s "${ROOTFS_IMG}")"
echo "rootfs 镜像: ${ROOTFS_IMG} (${ROOTFS_SIZE} 字节)"

echo "=== Step 2: 基于源镜像创建新镜像 ==="

echo "源镜像: ${SRC_IMAGE}"
echo "新镜像: ${OUT_IMAGE}"
cp "${SRC_IMAGE}" "${OUT_IMAGE}"

echo "=== Step 3: 写入 rootfs 到新镜像 ext4 分区 ==="

dd if="${ROOTFS_IMG}" of="${OUT_IMAGE}" bs=512 seek=${ROOTFS_SECTOR} conv=notrunc status=progress

echo "=== Step 4: 验证新镜像 ==="
PART_OFF=$((ROOTFS_SECTOR * 512))
if debugfs -R "stat /bin/sh" "${OUT_IMAGE}?offset=${PART_OFF}" 2>/dev/null; then
    echo "/bin/sh 存在: OK"
else
    echo "警告: 无法读取新镜像中的 /bin/sh"
fi

echo ""
echo "=== 完成 ==="
echo "新镜像: ${OUT_IMAGE}"
echo "源镜像未修改: ${SRC_IMAGE}"
echo ""
echo "烧写: sudo dd if=${OUT_IMAGE} of=/dev/mmcblk0 bs=4M status=progress conv=fsync"
