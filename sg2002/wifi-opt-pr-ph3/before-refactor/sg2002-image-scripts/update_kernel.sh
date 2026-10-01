#!/usr/bin/env bash
#
# update_kernel.sh — 编译新 StarryOS 内核并生成新 SD 卡镜像
#
# 基于已有镜像创建新镜像，替换其中的 StarryOS 内核（boot.sd FIT image）。
# 原镜像不做任何修改，产出为独立的新镜像文件。
#
# 用法:
#   ./www/sg2002-image-scripts/update_kernel.sh [SRC_IMAGE] [OUT_IMAGE]
#
#   SRC_IMAGE  源镜像路径（不会被修改），默认为 target/sg2002/sg2002_starryos_wifi.img
#   OUT_IMAGE  新镜像路径，默认在源镜像同目录生成带时间戳的文件名
#
# 需要从仓库根目录执行。
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

WORK_DIR="target/sg2002"
BOARD_CFG_DIR="os/StarryOS/configs/board"
DTB_SRC="${BOARD_CFG_DIR}/licheerv-nano-sg2002.dtb"
WIFI_TOML="${BOARD_CFG_DIR}/licheerv-nano-sg2002-wifi.toml"
DEFAULT_TOML="${BOARD_CFG_DIR}/licheerv-nano-sg2002.toml"
DEFAULT_ITS="${BOARD_CFG_DIR}/licheerv-nano-sg2002.its"
WIFI_ITS="${BOARD_CFG_DIR}/licheerv-nano-sg2002-wifi.its"
STARRY_BIN="target/riscv64gc-unknown-none-elf/release/starryos.bin"

SRC_IMAGE="${1:-${WORK_DIR}/sg2002_starryos_wifi.img}"

if [[ ! -f "${SRC_IMAGE}" ]]; then
    echo "错误: 源镜像不存在: ${SRC_IMAGE}"
    echo "用法: $0 [SRC_IMAGE] [OUT_IMAGE]"
    exit 1
fi

# 默认在源镜像同目录生成带时间戳的新文件名
if [[ $# -ge 2 ]]; then
    OUT_IMAGE="$2"
else
    SRC_DIR="$(dirname "${SRC_IMAGE}")"
    SRC_BASE="$(basename "${SRC_IMAGE}" .img)"
    TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
    OUT_IMAGE="${SRC_DIR}/${SRC_BASE}_kernel-${TIMESTAMP}.img"
fi

if [[ -f "${OUT_IMAGE}" ]]; then
    echo "错误: 输出镜像已存在: ${OUT_IMAGE}"
    exit 1
fi

BOOT_SECTOR=2048

echo "=== Step 1: 编译 StarryOS 内核 ==="

cp "${DEFAULT_TOML}" "${DEFAULT_TOML}.bak"
cp "${WIFI_TOML}" "${DEFAULT_TOML}"
cp "${DEFAULT_ITS}" "${DEFAULT_ITS}.bak" 2>/dev/null || true
cp "${WIFI_ITS}" "${DEFAULT_ITS}" 2>/dev/null || true
trap 'mv "${DEFAULT_TOML}.bak" "${DEFAULT_TOML}"; mv "${DEFAULT_ITS}.bak" "${DEFAULT_ITS}" 2>/dev/null || true' EXIT

cargo xtask starry quick-start licheerv-nano-sg2002 build

mv "${DEFAULT_TOML}.bak" "${DEFAULT_TOML}"
mv "${DEFAULT_ITS}.bak" "${DEFAULT_ITS}" 2>/dev/null || true
trap - EXIT

echo "=== Step 2: 验证必要资产 ==="

if [[ ! -f "${WORK_DIR}/ramdisk.bin" ]]; then
    echo "错误: ${WORK_DIR}/ramdisk.bin 不存在。"
    echo "ramdisk.bin 是生成 boot.sd 的必要组件，请先从 SG2002 Linux 镜像提取。"
    echo "参考: www/sg2002-starryos-image-build-guide.md 步骤二"
    exit 1
fi

echo "=== Step 3: 准备 DTB 和内核 ==="

mkdir -p "${WORK_DIR}"
cp "${DTB_SRC}" "${WORK_DIR}/"
cp "${STARRY_BIN}" "${WORK_DIR}/starryos.bin"

echo "=== Step 4: 重新生成 boot.sd ==="

cat > "${WORK_DIR}/boot.its" <<'ITS_EOF'
/dts-v1/;

/ {
    description = "StarryOS WiFi kernel for SG2002 LicheeRV Nano";
    #address-cells = <1>;

    images {
        kernel-1 {
            description = "StarryOS WiFi kernel";
            data = /incbin/("starryos.bin");
            type = "kernel";
            arch = "riscv";
            os = "linux";
            compression = "none";
            load = <0x80200000>;
            entry = <0x80200000>;
            hash-1 {
                algo = "crc32";
            };
        };

        ramdisk-1 {
            description = "cvitek ramdisk";
            data = /incbin/("ramdisk.bin");
            type = "ramdisk";
            arch = "riscv";
            os = "linux";
            compression = "none";
            hash-1 {
                algo = "crc32";
            };
        };

        fdt-sg2002_licheervnano_sd {
            description = "cvitek device tree - sg2002_licheervnano_sd";
            data = /incbin/("licheerv-nano-sg2002.dtb");
            type = "flat_dt";
            arch = "riscv";
            compression = "none";
            hash-1 {
                algo = "sha256";
            };
        };
    };

    configurations {
        default = "config-sg2002_licheervnano_sd";
        config-sg2002_licheervnano_sd {
            description = "StarryOS WiFi boot for sg2002_licheervnano_sd";
            kernel = "kernel-1";
            ramdisk = "ramdisk-1";
            fdt = "fdt-sg2002_licheervnano_sd";
        };
    };
};
ITS_EOF

(
    cd "${WORK_DIR}"
    mkimage -f boot.its boot.sd
)

echo "=== Step 5: 基于源镜像创建新镜像 ==="

echo "源镜像: ${SRC_IMAGE}"
echo "新镜像: ${OUT_IMAGE}"
cp "${SRC_IMAGE}" "${OUT_IMAGE}"

BOOT_OFF=$((BOOT_SECTOR * 512))
mcopy -i "${OUT_IMAGE}@@${BOOT_OFF}" -o "${WORK_DIR}/boot.sd" ::boot.sd

echo "=== Step 6: 验证新镜像 ==="
mcopy -i "${OUT_IMAGE}@@${BOOT_OFF}" ::boot.sd /tmp/check_new.sd
mkimage -l /tmp/check_new.sd
rm -f /tmp/check_new.sd

echo ""
echo "=== 完成 ==="
echo "新镜像: ${OUT_IMAGE}"
echo "源镜像未修改: ${SRC_IMAGE}"
echo ""
echo "烧写: sudo dd if=${OUT_IMAGE} of=/dev/mmcblk0 bs=4M status=progress conv=fsync"
