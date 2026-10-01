#!/usr/bin/env bash
#
# build-tf-image.sh — Assemble SG2002 StarryOS WiFi SD card image with iperf3 rootfs
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

WORK_DIR="$REPO_ROOT/target/sg2002"
BOARD_CFG_DIR="$REPO_ROOT/os/StarryOS/configs/board"
DTB_SRC="$REPO_ROOT/os/StarryOS/configs/board/licheerv-nano-sg2002.dtb"
STARRY_BIN="$REPO_ROOT/target/riscv64gc-unknown-none-elf/release/starryos.bin"
ROOTFS_IMG="$REPO_ROOT/target/sg2002/rootfs-riscv64-alpine-iperf3.img"
OUT_NAME="${1:-sg2002_starryos_wifi_iperf3}"

BOOT_SECTOR=2048
BOOT_SIZE_SECTORS=131072
ROOTFS_SECTOR=133120
ROOTFS_SIZE_SECTORS=4198400
IMG_SIZE_SECTORS=4333568

echo "=== Step 1: Verify components ==="
mkdir -p "${WORK_DIR}"

for f in "${WORK_DIR}/fip.bin" "${WORK_DIR}/ramdisk.bin" "${STARRY_BIN}" "${DTB_SRC}" "${ROOTFS_IMG}"; do
    if [[ ! -f "$f" ]]; then
        echo "ERROR: Missing $f"
        exit 1
    fi
    echo "  OK: $f ($(stat -c%s "$f") bytes)"
done

echo "=== Step 2: Copy DTB and kernel ==="
cp "${DTB_SRC}" "${WORK_DIR}/licheerv-nano-sg2002.dtb"
cp "${STARRY_BIN}" "${WORK_DIR}/starryos.bin"

echo "=== Step 3: Generate FIT Image (boot.sd) ==="
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
    echo "boot.sd created: $(stat -c%s boot.sd) bytes"
)

echo "=== Step 4: Assemble SD card image ==="
OUT="${WORK_DIR}/${OUT_NAME}.img"

(
    cd "${WORK_DIR}"

    # Create empty image (2.1 GB sparse)
    dd if=/dev/zero of="${OUT_NAME}.img" bs=512 count=0 seek=${IMG_SIZE_SECTORS}

    # Create MBR partition table
    sfdisk "${OUT_NAME}.img" <<SFDISK_EOF
label: dos
unit: sectors
start=${BOOT_SECTOR}, size=${BOOT_SIZE_SECTORS}, type=c, bootable
start=${ROOTFS_SECTOR}, size=${ROOTFS_SIZE_SECTORS}, type=83
SFDISK_EOF

    # Format boot partition and write fip.bin + boot.sd
    mkfs.fat -F 32 -n BOOT --offset ${BOOT_SECTOR} "${OUT_NAME}.img"
    BOOT_OFF=$((BOOT_SECTOR * 512))
    mcopy -i "${OUT_NAME}.img@@${BOOT_OFF}" fip.bin ::
    mcopy -i "${OUT_NAME}.img@@${BOOT_OFF}" boot.sd ::

    # Write rootfs (dd to ext4 partition, no format needed)
    dd if="${ROOTFS_IMG}" of="${OUT_NAME}.img" bs=512 seek=${ROOTFS_SECTOR} conv=notrunc status=progress
)

echo ""
echo "=== Step 5: Verify ==="
BOOT_OFF=$((BOOT_SECTOR * 512))
PART_OFF=$((ROOTFS_SECTOR * 512))

echo "Partition table:"
fdisk -l "${OUT}" 2>/dev/null | grep -E "Device|${OUT_NAME}" || true

echo ""
echo "boot partition contents:"
mdir -i "${OUT}@@${BOOT_OFF}" ::

echo ""
echo "/usr/bin/iperf3 in rootfs:"
debugfs -R "stat /usr/bin/iperf3" "${OUT}?offset=${PART_OFF}" 2>/dev/null | grep -E "Inode|Size|Mode" || echo "  WARNING: iperf3 not found"

echo ""
echo "=== Done ==="
echo "Image: ${OUT}"
echo "Size: $(stat -c%s "${OUT}") bytes"
echo ""
echo "Write to SD card:"
echo "  sudo dd if=${OUT} of=/dev/mmcblk0 bs=4M status=progress conv=fsync"
