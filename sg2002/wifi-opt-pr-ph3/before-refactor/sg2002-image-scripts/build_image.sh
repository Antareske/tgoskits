#!/usr/bin/env bash
#
# build_image.sh — 完整构建 SG2002 StarryOS WiFi SD 卡镜像
#
# 从零开始编译内核并组装 2.1 GB 可启动 SD 卡镜像。
# 需要从仓库根目录执行。
#
# 用法:
#   ./www/sg2002-image-scripts/build_image.sh [NAME]
#
#   NAME  输出镜像名（不含 .img 后缀），默认为 sg2002_starryos_wifi
#
# 产出:
#   target/sg2002/<NAME>.img
#
# 预置条件:
#   - target/sg2002/fip.bin (440832 字节)
#   - target/sg2002/ramdisk.bin (2527648 字节)
#   若缺失，脚本会提示来源并退出。
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
ROOTFS_IMG="tmp/axbuild/rootfs/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img"
OUT_NAME="${1:-sg2002_starryos_wifi}"

FIP_SIZE=440832
RAMDISK_SIZE=2527648
BOOT_SECTOR=2048
BOOT_SIZE_SECTORS=131072
ROOTFS_SECTOR=133120
ROOTFS_SIZE_SECTORS=4198400
IMG_SIZE_SECTORS=4333568

echo "=== Step 1: 编译 StarryOS 内核 ==="

# 用 wifi 配置临时替换默认配置
cp "${DEFAULT_TOML}" "${DEFAULT_TOML}.bak"
cp "${WIFI_TOML}" "${DEFAULT_TOML}"
cp "${DEFAULT_ITS}" "${DEFAULT_ITS}.bak" 2>/dev/null || true
cp "${WIFI_ITS}" "${DEFAULT_ITS}" 2>/dev/null || true
trap 'mv "${DEFAULT_TOML}.bak" "${DEFAULT_TOML}"; mv "${DEFAULT_ITS}.bak" "${DEFAULT_ITS}" 2>/dev/null || true' EXIT

cargo xtask starry quick-start licheerv-nano-sg2002 build

# 恢复配置
mv "${DEFAULT_TOML}.bak" "${DEFAULT_TOML}"
mv "${DEFAULT_ITS}.bak" "${DEFAULT_ITS}" 2>/dev/null || true
trap - EXIT

echo "=== Step 2: 准备 fip.bin 和 ramdisk.bin ==="

mkdir -p "${WORK_DIR}"

if [[ ! -f "${WORK_DIR}/fip.bin" ]]; then
    echo "错误: ${WORK_DIR}/fip.bin 不存在。"
    echo "请从 SG2002 Linux 镜像提取，参考: www/sg2002-starryos-image-build-guide.md 步骤二"
    exit 1
fi

FIP_ACTUAL="$(stat -c%s "${WORK_DIR}/fip.bin")"
if [[ "${FIP_ACTUAL}" -ne "${FIP_SIZE}" ]]; then
    echo "警告: fip.bin 大小为 ${FIP_ACTUAL} 字节，预期 ${FIP_SIZE} 字节"
fi

if [[ ! -f "${WORK_DIR}/ramdisk.bin" ]]; then
    # 尝试从已有 boot.sd 提取
    if [[ -f "${WORK_DIR}/boot.sd" ]]; then
        echo "从已有 boot.sd 提取 ramdisk.bin ..."
        dumpimage -T flat_dt -p 1 -o "${WORK_DIR}/ramdisk.bin" "${WORK_DIR}/boot.sd"
    else
        echo "错误: ${WORK_DIR}/ramdisk.bin 不存在，也未找到 boot.sd 用于提取。"
        echo "请从 SG2002 Linux 镜像提取，参考: www/sg2002-starryos-image-build-guide.md 步骤二"
        exit 1
    fi
fi

RAMDISK_ACTUAL="$(stat -c%s "${WORK_DIR}/ramdisk.bin")"
if [[ "${RAMDISK_ACTUAL}" -ne "${RAMDISK_SIZE}" ]]; then
    echo "警告: ramdisk.bin 大小为 ${RAMDISK_ACTUAL} 字节，预期 ${RAMDISK_SIZE} 字节"
fi

echo "=== Step 3: 准备 DTB ==="

cp "${DTB_SRC}" "${WORK_DIR}/"
echo "已复制项目 DTB (${DTB_SRC}) → ${WORK_DIR}/licheerv-nano-sg2002.dtb"

echo "=== Step 4: 生成 FIT Image (boot.sd) ==="

cp "${STARRY_BIN}" "${WORK_DIR}/starryos.bin"

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

echo "=== Step 5: 准备 Rootfs ==="

cargo xtask starry rootfs --arch riscv64

if [[ ! -f "${ROOTFS_IMG}" ]]; then
    echo "错误: rootfs 镜像未找到: ${ROOTFS_IMG}"
    exit 1
fi

echo "=== Step 6: 组装 SD 卡镜像 ==="

OUT="${WORK_DIR}/${OUT_NAME}.img"

(
    cd "${WORK_DIR}"

    dd if=/dev/zero of="${OUT##*/}" bs=512 count=0 seek=${IMG_SIZE_SECTORS}

    sfdisk "${OUT##*/}" <<SFDISK_EOF
label: dos
unit: sectors
start=${BOOT_SECTOR}, size=${BOOT_SIZE_SECTORS}, type=c, bootable
start=${ROOTFS_SECTOR}, size=${ROOTFS_SIZE_SECTORS}, type=83
SFDISK_EOF

    mkfs.fat -F 32 -n BOOT --offset ${BOOT_SECTOR} "${OUT##*/}"
    BOOT_OFF=$((BOOT_SECTOR * 512))
    mcopy -i "${OUT##*/}@@${BOOT_OFF}" fip.bin ::
    mcopy -i "${OUT##*/}@@${BOOT_OFF}" boot.sd ::

    ROOTFS_ABS="${REPO_ROOT}/${ROOTFS_IMG}"
    dd if="${ROOTFS_ABS}" of="${OUT##*/}" bs=512 seek=${ROOTFS_SECTOR} conv=notrunc status=progress
)

echo ""
echo "=== 完成 ==="
echo "SD 卡镜像: ${WORK_DIR}/${OUT_NAME}.img"
echo ""
echo "烧写: sudo dd if=${WORK_DIR}/${OUT_NAME}.img of=/dev/mmcblk0 bs=4M status=progress conv=fsync"
