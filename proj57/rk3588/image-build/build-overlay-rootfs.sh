#!/usr/bin/env bash
# build-overlay-rootfs.sh - 把 ACT 推理 overlay 叠加到 StarryOS 基础 rootfs，
# 产出功能齐全且 fsck 干净的 starry-rootfs-act-infer.ext4。
#
# 为什么需要这个脚本（固化的关键点）：
#   ACT 二进制以 aarch64-unknown-linux-gnu（glibc）目标交叉编译（librknnrt.so 依赖
#   glibc 符号），而 StarryOS 基础 rootfs 是 Alpine/musl，只带 musl 运行时。仅把
#   install 目录拷进 /act_infer_rk3588 并不够：
#     1. 二进制的 ELF 解释器写死为绝对路径 /lib/ld-linux-aarch64.so.1，内核 exec 时
#        按此绝对路径找加载器，$ORIGIN/lib rpath 与 LD_LIBRARY_PATH 都改不了它。
#        基础 rootfs 没有这个文件，必须在 rootfs 的 /lib 下放 glibc 加载器。
#     2. glibc 共享库（libc/libm/libgcc_s/libpthread/libdl/libstdc++）必须随程序
#        一起进 /act_infer_rk3588/lib（由 $ORIGIN/lib rpath 解析）。
#   build-rk3588.sh 已把上述 glibc 运行时打进 install 目录的 lib/；本脚本负责把它
#   合进 rootfs，并把加载器接到 /lib/ld-linux-aarch64.so.1，最后逐项校验，避免漏装
#   导致板上 exec 直接失败（表现为 not found / 动态链接错误，而非推理逻辑问题）。
#
# 实现方式（无需 root/loop mount）：
#   debugfs 的 `write` 不更新 inode 的 i_blocks，产出的 ext4 在 fsck 看来是脏的，
#   板上启动期 fsck 可能把这些文件当未分配块清零。改用确定性更强的链路：
#     fakeroot 下 `debugfs rdump` 解出基础 rootfs 到暂存目录（保留 root 属主/权限）
#     → 叠加 install 目录与 glibc 加载器软链 → `mke2fs -d` 由暂存目录重建干净 ext4。
#   全程在 fakeroot 内完成，属主记为 root，且无需真实 root 权限。
#
# 用法:
#   ./build-overlay-rootfs.sh [输出ext4路径]
# 环境变量:
#   BASE_ROOTFS   StarryOS 基础 rootfs ext4 (默认自动探测 tgoskits 下载产物)
#   INSTALL_DIR   ACT install 目录 (默认 ../install/rk3588_linux_aarch64/act_infer_rk3588)
#   DEST          rootfs 内安装路径 (默认 /act_infer_rk3588)
#   MARGIN_MB     文件系统在内容之上的余量 MiB (默认 128)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

OUT="${1:-$HERE/starry-rootfs-act-infer.ext4}"
APP_DIR="$(cd "$HERE/.." && pwd)"
: "${INSTALL_DIR:=$APP_DIR/install/rk3588_linux_aarch64/act_infer_rk3588}"
: "${DEST:=/act_infer_rk3588}"
: "${MARGIN_MB:=128}"

for t in debugfs mke2fs e2fsck fakeroot stat du; do
  command -v "$t" >/dev/null 2>&1 || { echo "缺少工具 $t (安装 e2fsprogs / fakeroot)" >&2; exit 1; }
done

# 1) 解析基础 rootfs
if [ -z "${BASE_ROOTFS:-}" ]; then
  for cand in \
    /tmp/.tgos-images/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img \
    "$APP_DIR/../../tmp/axbuild/rootfs/rootfs-aarch64-alpine.img/rootfs-aarch64-alpine.img"; do
    [ -f "$cand" ] && { BASE_ROOTFS="$cand"; break; }
  done
fi
[ -n "${BASE_ROOTFS:-}" ] && [ -f "$BASE_ROOTFS" ] || {
  echo "找不到基础 rootfs。先跑 'cargo xtask starry rootfs --arch aarch64'，" >&2
  echo "或用 BASE_ROOTFS= 指定。" >&2; exit 1; }

[ -d "$INSTALL_DIR" ] || {
  echo "找不到 install 目录: $INSTALL_DIR" >&2
  echo "先跑 ../scripts/build-rk3588.sh（它会打包二进制+模型+glibc 运行时）。" >&2
  exit 1; }

# install 目录必须自带 glibc 运行时（build-rk3588.sh 负责打包）
need_in_install=(
  act-infer-golden-rknn act-infer-review-rknn
  run-golden.sh run-review.sh
  lib/librknnrt.so lib/ld-linux-aarch64.so.1
  lib/libc.so.6 lib/libm.so.6 lib/libgcc_s.so.1
  lib/libpthread.so.0 lib/libdl.so.2 lib/libstdc++.so.6
  model/model.rknn model/stats.json model/golden.json
  model/input.jpg model/input_state.bin
  model/review_left.jpg model/review_right.jpg
)
miss=()
for rel in "${need_in_install[@]}"; do
  [ -e "$INSTALL_DIR/$rel" ] || miss+=("$rel")
done
if [ ${#miss[@]} -gt 0 ]; then
  echo "install 目录缺少以下资产/运行时:" >&2
  printf '  %s\n' "${miss[@]}" >&2
  echo "请重跑 ../scripts/build-rk3588.sh（确保 glibc 交叉运行时已安装）。" >&2
  exit 1
fi

# 基础 rootfs 必须是干净的 musl rootfs（不含 $DEST）。若已包含则说明是上次运行
# 的产物被误作基底，重新下载基础 rootfs 再跑本脚本。
if debugfs -R "stat $DEST" "$BASE_ROOTFS" 2>&1 | grep -q "Inode:"; then
  echo "错误: BASE_ROOTFS ($BASE_ROOTFS) 已包含 $DEST，不是干净的基础 rootfs。" >&2
  echo "请重新下载基础 rootfs 后再跑:" >&2
  echo "  cargo xtask starry rootfs --arch aarch64" >&2
  exit 1
fi

echo ">> 基础 rootfs : $BASE_ROOTFS"
echo ">> install 目录: $INSTALL_DIR"
echo ">> 输出 ext4   : $OUT"

stage="$(mktemp -d)"
fr_log="$(mktemp)"
trap 'rm -rf "$stage" "$fr_log"' EXIT

# 2) 在 fakeroot 内: 解出基础 rootfs -> 叠加 overlay -> 重建干净 ext4
#    所有步骤都在同一个 fakeroot 进程内，属主记账连续（基础文件保持 root, 新加文件也 root）。
fakeroot -- bash -s -- "$BASE_ROOTFS" "$INSTALL_DIR" "$DEST" "$stage" "$OUT" "$MARGIN_MB" <<'FAKEROOT_EOF'
set -euo pipefail
base="$1"; install_dir="$2"; dest="$3"; stage="$4"; out="$5"; margin_mb="$6"

# 解出基础 rootfs（rdump 保留属主/权限/符号链接；在 fakeroot 下 root 属主可还原）
debugfs -R "rdump / ${stage}" "$base" >/dev/null 2>&1

# 叠加 ACT install 目录到 ${dest}
rm -rf "${stage}${dest}"
cp -a "$install_dir" "${stage}${dest}"

# glibc 加载器接到 rootfs 的 /lib/ld-linux-aarch64.so.1（ELF 解释器绝对路径）。
# 基础 musl rootfs 没有此文件；用绝对软链指向 app 目录里的加载器，让 glibc
# 由 app 目录单一来源管理。基础 musl 加载器 /lib/ld-musl-aarch64.so.1 保持不动。
ln -sf "${dest}/lib/ld-linux-aarch64.so.1" "${stage}/lib/ld-linux-aarch64.so.1"

# 计算文件系统大小: 暂存内容 + 余量
content_mb=$(( $(du -sm "$stage" | awk '{print $1}') ))
fs_mb=$(( content_mb + margin_mb ))

# 由暂存目录重建干净 ext4。-O ^metadata_csum 与基础 rootfs 一致，避免某些
# 旧内核/工具链对 csum 处理差异。卷标 starry-rootfs 与 GPT 标签/命令行无关，
# 真正的 PARTLABEL 由 build-image.sh 的 GPT 分区名决定。
rm -f "$out"
mke2fs -q -t ext4 -L starry-rootfs -O ^metadata_csum -d "$stage" "$out" "${fs_mb}M"
FAKEROOT_EOF

# 3) 校验文件系统干净 + overlay 功能齐全
echo ">> 校验文件系统一致性 (e2fsck -fn):"
if e2fsck -fn "$OUT" >/dev/null 2>&1; then
  echo "  [ok] 文件系统干净"
else
  echo "  [警告] e2fsck 报告文件系统不一致" >&2
  e2fsck -fn "$OUT" 2>&1 | sed -n '2,12p' >&2
  exit 1
fi

echo ">> 校验 overlay:"
fail=0
check() { # <debugfs路径> <说明> [min_bytes]
  local path="$1" desc="$2" min="${3:-0}"
  local info
  info="$(debugfs -R "stat $path" "$OUT" 2>/dev/null)" || { echo "  [缺失] $path  ($desc)" >&2; fail=1; return; }
  if [ "$min" -gt 0 ]; then
    local sz
    sz="$(printf '%s' "$info" | grep -oP '(?<=Size: )\d+' | head -1)"
    [ -n "$sz" ] || sz=0
    if [ "$sz" -lt "$min" ]; then
      echo "  [空/损坏] $path  ($desc, size=${sz} < ${min})" >&2; fail=1; return
    fi
  fi
  echo "  [ok] $path"
}
check "$DEST/act-infer-golden-rknn"   "golden 二进制" 100000
check "$DEST/act-infer-review-rknn"   "review 二进制" 100000
check "$DEST/run-golden.sh"           "golden 启动器" 100
check "$DEST/run-review.sh"           "review 启动器" 100
check "$DEST/lib/librknnrt.so"        "RKNPU2 runtime" 1000000
check "$DEST/lib/libc.so.6"           "glibc libc" 100000
check "$DEST/lib/libstdc++.so.6"      "glibc libstdc++"
check "$DEST/lib/libgcc_s.so.1"       "glibc libgcc" 10000
check "$DEST/lib/ld-linux-aarch64.so.1" "app 内 glibc 加载器" 100000
check "/lib/ld-linux-aarch64.so.1"    "rootfs glibc 加载器(ELF 解释器路径)"
check "/lib/ld-musl-aarch64.so.1"     "基础 musl 加载器(保持原样)"
check "$DEST/model/model.rknn"        "RKNN 模型" 1000000
check "$DEST/model/stats.json"        "归一化统计量" 10
check "$DEST/model/golden.json"       "golden 基准" 10
check "$DEST/model/review_left.jpg"   "review 左用例图" 100
check "$DEST/model/review_right.jpg"  "review 右用例图" 100

if [ "$fail" -ne 0 ]; then
  echo ">> 校验未通过，overlay 不完整。" >&2
  exit 1
fi
echo ">> overlay 完整且 fsck 干净。下一步合成整盘镜像:"
echo "   ./make-dtb.sh && ./repack-fit.sh"
echo "   ROOTFS=$(basename "$OUT") ./build-image.sh output/rk3588-starryos-act-infer.img"
