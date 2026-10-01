#!/bin/sh
# 出 SG2002（LicheeRV Nano）板测镜像：本分支的板级内核 + 派生 DTB + 镜像组装。
#
# 与镜像工具默认产物相比有三处不同，每一处都是新基线 dev（含 #2528 的
# `feat(initramfs): unify host image boot flow`）引入的板级回归，理由如下：
#
#   1. DTB 去掉 /chosen/linux,initrd-*，并补 32 字节 rng-seed。
#      前者：那个提交之后内核把该区间当 host initramfs 严格校验，而这个区间是
#      DTB 自己声明的（官方 Linux 流程里那地址恒定放 ramdisk），本板那块地址上
#      没有 host 镜像，于是启动 panic。后者：WPA2 握手要的启动熵，
#      someboot 要求 /chosen/rng-seed 恰好 32 字节，而主线那份 DTB 没有这一项；
#      上游把熵注入只接在 `starry run` 板卡路径上，裸构建得自己补。
#   2. FIT 里不放 ramdisk，使那段地址不再是任何东西的落点。
#   3. rootfs 的 init 取自 dev 自己的 rootfs：基准镜像的 p2 沿自另一条链，用的是
#      原版 Alpine inittab，其中 tty1..tty6 的 getty 会无限 respawn（StarryOS 没有
#      虚拟控制台），控制台因此拿不到 shell。dev rootfs 用
#      /usr/libexec/starry/console 取代了那六条。
#
# 另有一件与镜像无关但决定 WiFi 能不能起的事：STA 凭据是编译期编进驱动的
# （drivers/ax-driver/build.rs 读 STARRY_WIFI_SSID / STARRY_WIFI_PASSWORD，
# 用 option_env! 进 aic8800 的 startup_config），关联由驱动自己发起，rootfs 里
# 不需要 wpa_supplicant。缺凭据时内核照常起、wlan0 也 UP，但永远不关联。
#
# 用法（必须显式给出这些输入）：
#   BASE_IMAGE=<基准镜像.img> OUT_IMAGE=<输出.img> IMAGE_TOOL=<sg2002-image-build 目录> \
#   STARRY_WIFI_SSID=<ssid> STARRY_WIFI_PASSWORD=<password> \
#       sh www/sg2002/wifi-sta/build-board-image.sh
#
# 可选：
#   DEV_ROOTFS    dev rootfs 镜像，取初始化文件用
#   BOARD_CONFIG  板级构建配置（默认本目录的对数版配置）
#   AIC8800_FIRMWARE_DIR  固件本地缓存目录，避免构建期联网取值
#   KERNEL        内核产物路径（默认 <本树>/target/.../release/starryos.bin）
#   SKIP_KERNEL=1 复用 KERNEL 指向的内核，不重新编译（仍会核对凭据）
#   IN_PLACE=1    就地改写 BASE_IMAGE（此时不必给 OUT_IMAGE）

set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
TREE=$(cd "$HERE/../../.." && pwd)

: "${BASE_IMAGE:?set BASE_IMAGE to the base image to copy}"
: "${IMAGE_TOOL:?set IMAGE_TOOL to the sg2002-image-build directory}"
# 凭据是编译期进驱动的：缺了不是"WiFi 弱一点"，而是根本不关联。
: "${STARRY_WIFI_SSID:?set STARRY_WIFI_SSID (compile-time STA credentials)}"
: "${STARRY_WIFI_PASSWORD:?set STARRY_WIFI_PASSWORD (compile-time STA credentials)}"
export STARRY_WIFI_SSID STARRY_WIFI_PASSWORD
: "${DEV_ROOTFS:?set DEV_ROOTFS to the dev rootfs image (init files)}"
: "${BOARD_CONFIG:=$HERE/licheerv-nano-sg2002-wifi-error.toml}"

if [ "${IN_PLACE:-0}" = 1 ]; then
    OUT_IMAGE="$BASE_IMAGE"
else
    : "${OUT_IMAGE:?set OUT_IMAGE (or IN_PLACE=1 to overwrite BASE_IMAGE)}"
fi

TOOL_SH="$IMAGE_TOOL/scripts/sg2002-image-build.sh"
INJECT_SH="$IMAGE_TOOL/scripts/inject-rootfs.sh"
KERNEL=${KERNEL:-$TREE/target/riscv64gc-unknown-none-elf/release/starryos.bin}
SRC_DTB="$TREE/os/StarryOS/configs/board/licheerv-nano-sg2002.dtb"
WORK=${WORK:-$TREE/.sg2002-build}

# p2 的起始字节偏移（扇区 133120 × 512），注入与校验都用它。
P2=68157440
# 基准镜像自带的 iperf3（rootfs 与 opt 线一致的那两件），出镜像前核对仍在。
IPERF3_LIB=/usr/lib/libiperf.so.0

for f in "$TOOL_SH" "$INJECT_SH" "$BASE_IMAGE" "$SRC_DTB" "$DEV_ROOTFS" "$BOARD_CONFIG"; do
    [ -e "$f" ] || { echo "缺少: $f" >&2; exit 1; }
done

cd "$TREE"

mkdir -p "$WORK/assets"
for a in ramdisk.bin fip.bin; do
    [ -e "$WORK/assets/$a" ] || { echo "缺少资产 $WORK/assets/$a（repack-fit/组装需要）" >&2; exit 1; }
done

if [ "${SKIP_KERNEL:-0}" != 1 ]; then
    echo "== 编译板级内核（$BOARD_CONFIG） =="
    echo "   STA 凭据: ssid=$STARRY_WIFI_SSID"
    cargo xtask starry build -c "${BOARD_CONFIG#"$TREE"/}"
fi
[ -f "$KERNEL" ] || { echo "没有内核产物: $KERNEL" >&2; exit 1; }

# 出镜像前的一道闸：凭据必须真的编进去了。
if ! strings "$KERNEL" | grep -qF "$STARRY_WIFI_SSID"; then
    echo "内核里找不到 SSID '$STARRY_WIFI_SSID'：这份内核不是带凭据构建的。" >&2
    echo "用 STARRY_WIFI_SSID / STARRY_WIFI_PASSWORD 重新编译（不要用 SKIP_KERNEL）。" >&2
    exit 1
fi
echo "  [ok] 内核里含 SSID '$STARRY_WIFI_SSID'"

echo "== 派生 DTB =="
DTB="$WORK/assets/licheerv-nano-sg2002-noinitrd.dtb"
cp "$SRC_DTB" "$DTB"
fdtput -d "$DTB" /chosen linux,initrd-start linux,initrd-end
SEED=$(od -An -tx4 -N32 /dev/urandom | tr -s ' ' '\n' | grep -E '^[0-9a-f]{8}$' | head -8 | tr '\n' ' ')
[ "$(echo "$SEED" | wc -w)" -eq 8 ] || { echo "随机种子生成失败" >&2; exit 1; }
fdtput -t x "$DTB" /chosen rng-seed $SEED
echo "  rng-seed: $SEED"

echo "== 组装镜像 =="
export ITS="$HERE/boot-noinitrd.its"
if [ "$OUT_IMAGE" = "$BASE_IMAGE" ]; then
    "$TOOL_SH" update-kernel "$OUT_IMAGE" --work "$WORK" --kernel "$KERNEL" --dtb "$DTB" --overwrite
else
    "$TOOL_SH" update-kernel "$BASE_IMAGE" --work "$WORK" --kernel "$KERNEL" --dtb "$DTB" -o "$OUT_IMAGE"
fi

echo "== 取 dev rootfs 的初始化文件 =="
INIT="$WORK/init-files"
mkdir -p "$INIT"
debugfs -R "dump /etc/inittab $INIT/inittab" "$DEV_ROOTFS" >/dev/null
debugfs -R "dump /usr/libexec/starry/console $INIT/console" "$DEV_ROOTFS" >/dev/null
debugfs -R "dump /etc/profile.d/starry.sh $INIT/starry.sh" "$DEV_ROOTFS" >/dev/null

echo "== 注入 p2 =="
"$INJECT_SH" "${OUT_IMAGE}?offset=${P2}" \
    --inject "$INIT/inittab:/etc/inittab" \
    --inject "$INIT/console:/usr/libexec/starry/console:0755" \
    --inject "$INIT/starry.sh:/etc/profile.d/starry.sh"

echo "== 校验 =="
mcopy -i "${OUT_IMAGE}@@1048576" ::boot.sd "$WORK/boot.sd"
if mkimage -l "$WORK/boot.sd" | grep -qi ramdisk; then
    echo "  [失败] FIT 里仍有 ramdisk" >&2; exit 1
fi
echo "  [ok] FIT 无 ramdisk"
mkimage -l "$WORK/boot.sd" | grep -E "^ *Description|Default Configuration" | sed 's/^/  /'
dumpimage -T flat_dt -p 0 -o "$WORK/k.bin" "$WORK/boot.sd" >/dev/null
dumpimage -T flat_dt -p 1 -o "$WORK/f.dtb" "$WORK/boot.sd" >/dev/null
cmp -s "$WORK/k.bin" "$KERNEL" || { echo "  [失败] FIT 内内核不是本树产物" >&2; exit 1; }
cmp -s "$WORK/f.dtb" "$DTB" || { echo "  [失败] FIT 内 DTB 不是派生件" >&2; exit 1; }
echo "  [ok] 内核与 DTB 都是本次构建的"
if dtc -I dtb -O dts "$WORK/f.dtb" 2>/dev/null | grep -q "linux,initrd"; then
    echo "  [失败] DTB 仍声明 initrd" >&2; exit 1
fi
if [ "$(dtc -I dtb -O dts "$WORK/f.dtb" 2>/dev/null | grep -c rng-seed)" -lt 1 ]; then
    echo "  [失败] DTB 无 rng-seed" >&2; exit 1
fi
echo "  [ok] /chosen 无 initrd 声明且带 rng-seed"

for pair in "inittab:/etc/inittab" "console:/usr/libexec/starry/console" "starry.sh:/etc/profile.d/starry.sh"; do
    want=${pair%%:*}; tgt=${pair#*:}
    rm -f "$WORK/back"
    debugfs -R "dump $tgt $WORK/back" "${OUT_IMAGE}?offset=${P2}" >/dev/null 2>&1
    cmp -s "$WORK/back" "$INIT/$want" || { echo "  [失败] $tgt 不一致" >&2; exit 1; }
done
echo "  [ok] p2：三个初始化文件逐字节一致"

if ! debugfs -R "stat /usr/bin/iperf3" "${OUT_IMAGE}?offset=${P2}" 2>/dev/null | grep -q "^Inode:"; then
    echo "  [失败] p2 里没有 /usr/bin/iperf3" >&2; exit 1
fi
if ! debugfs -R "stat $IPERF3_LIB" "${OUT_IMAGE}?offset=${P2}" 2>/dev/null | grep -q "^Inode:"; then
    echo "  [失败] p2 里没有 $IPERF3_LIB" >&2; exit 1
fi
echo "  [ok] p2：iperf3 与 $IPERF3_LIB 在位"
debugfs -R "stat /bin/sh" "${OUT_IMAGE}?offset=${P2}" 2>/dev/null | grep -q "^Inode:" \
    || { echo "  [失败] p2 不可读" >&2; exit 1; }
echo "  [ok] p2 可读"

echo "== 完成 =="
ls -la "$OUT_IMAGE"
sha256sum "$OUT_IMAGE" | cut -c1-16
