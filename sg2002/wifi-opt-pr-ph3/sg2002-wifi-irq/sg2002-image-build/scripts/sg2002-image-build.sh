#!/usr/bin/env bash
# sg2002-image-build.sh - SG2002 StarryOS SD 镜像构建/更新编排入口
#
# 在 tgoskits 工作树内执行。默认从当前工作树编译 starry、下载 rootfs,
# 在 <tgoskits根>/.sg2002-build/ 下组织资产与产物。详见 SKILL.md。
#
# 用法:
#   sg2002-image-build.sh provision <官方Linux镜像.img>
#   sg2002-image-build.sh build [选项]
#   sg2002-image-build.sh update-kernel <镜像.img> [选项]
#   sg2002-image-build.sh update-rootfs <镜像.img> [选项]
#   sg2002-image-build.sh inject [选项]
#   sg2002-image-build.sh check-deps
#   sg2002-image-build.sh clean
#
# 通用选项:
#   --source <path>     starry 编译来源 tgoskits 树 (默认当前工作树根)
#   --commit <rev>      在 --source 检出该提交编译 (git worktree add --detach)
#   --config <toml>     板级配置 (默认 os/StarryOS/configs/board/licheerv-nano-sg2002.toml)
#   --dtb <file>        覆盖 DTB
#   --work <dir>        工作目录 (默认 <tgoskits根>/.sg2002-build)
#   --kernel <bin>      复用已有内核, 跳过编译 (build/update-kernel)
#   --rootfs <ext4>     复用已有 rootfs, 跳过下载 (build/update-rootfs)
#   --inject host:target[:mode]  注入文件 (可重复)
#   --manifest <file>   注入清单 (可重复)
#   --init-assets       注入内置 init 套件 (sshd/WiFi 自启)
#   --overwrite         就地覆盖源镜像 (update-*)
#   --no-sync-uimg      update-kernel 不同步 /starryos.uimg (默认同步, 保持手动引导回退一致)
#   -o <out.img>        输出镜像路径 (默认 output/sg2002_starryos_<时间戳>.img)
set -euo pipefail

SKILL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPTS="$SKILL_ROOT/scripts"
TEMPLATES="$SKILL_ROOT/templates"

die() { echo "错误: $*" >&2; exit 1; }
warn() { echo "警告: $*" >&2; }

# 从当前目录向上找 tgoskits 工作树根 (含 os/StarryOS 与 Cargo.toml)
find_tgoskits_root() {
  local dir
  dir="$(pwd)"
  while [ "$dir" != "/" ]; do
    if [ -d "$dir/os/StarryOS" ] && [ -f "$dir/Cargo.toml" ]; then
      echo "$dir"; return 0
    fi
    dir="$(dirname "$dir")"
  done
  return 1
}

usage() {
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
}

cmd="${1:-}"; shift || true
case "$cmd" in
  provision|build|update-kernel|update-rootfs|inject|check-deps|clean) ;;
  -h|--help|"") usage; exit 0 ;;
  *) die "未知命令: $cmd"; usage >&2; exit 1 ;;
esac

SOURCE="" COMMIT="" CONFIG="" DTB="" WORK="" ROOTFS="" KERNEL="" OUT=""
INJECTS=() MANIFESTS=() POS=()
INIT_ASSETS=0 OVERWRITE=0 SYNC_UIMG=1

while [ $# -gt 0 ]; do
  case "$1" in
    --source)   SOURCE="${2:?--source 需要路径}"; shift 2 ;;
    --commit)   COMMIT="${2:?--commit 需要提交号}"; shift 2 ;;
    --config)   CONFIG="${2:?--config 需要 toml 路径}"; shift 2 ;;
    --dtb)      DTB="${2:?--dtb 需要文件路径}"; shift 2 ;;
    --work)     WORK="${2:?--work 需要目录路径}"; shift 2 ;;
    --kernel)   KERNEL="${2:?--kernel 需要文件路径}"; shift 2 ;;
    --rootfs)   ROOTFS="${2:?--rootfs 需要文件路径}"; shift 2 ;;
    --inject)   INJECTS+=("${2:?--inject 需要 host:target[:mode]}"); shift 2 ;;
    --manifest) MANIFESTS+=("${2:?--manifest 需要文件路径}"); shift 2 ;;
    --init-assets) INIT_ASSETS=1; shift ;;
    --overwrite)    OVERWRITE=1; shift ;;
    --no-sync-uimg) SYNC_UIMG=0; shift ;;
    -o)         OUT="${2:?-o 需要输出路径}"; shift 2 ;;
    -h|--help)  usage; exit 0 ;;
    *)          POS+=("$1"); shift ;;
  esac
done

# ---------- 基础环境 ----------

TG_ROOT="$(find_tgoskits_root || true)"
if [ -n "${WORK:-}" ]; then
  :
elif [ -n "${SOURCE:-}" ]; then
  WORK="$SOURCE/.sg2002-build"
elif [ -n "${TG_ROOT:-}" ]; then
  WORK="$TG_ROOT/.sg2002-build"
else
  # 不编译的命令 (inject/clean/update-*) 可以在任意目录配合 --work 使用
  WORK="$(pwd)/.sg2002-build"
fi

check_deps() {
  local missing=0
  for t in sfdisk mkfs.fat mcopy mdir mkimage dumpimage debugfs dd truncate stat git; do
    command -v "$t" >/dev/null 2>&1 || { echo "  缺少依赖: $t" >&2; missing=1; }
  done
  [ "$missing" -eq 0 ] || die "请先安装缺失的依赖工具 (详见 SKILL.md)"
  echo ">> 依赖工具齐全"
}

resolve_assets_dir() {
  mkdir -p "$WORK/assets" "$WORK/output" "$WORK/work"
  echo "$WORK"
}

# 编译来源: --commit 时在 source 建 detached worktree, 复用已建好的
resolve_build_src() {
  local src repo rev short sdir
  src="${SOURCE:-${TG_ROOT:-}}"
  if [ -n "${COMMIT:-}" ]; then
    [ -n "$src" ] || die "--commit 需要 --source 或在 tgoskits 工作树内执行"
    repo="$(git -C "$src" rev-parse --show-toplevel 2>/dev/null)" || die "--source 不是 git 仓库: $src"
    rev="$(git -C "$repo" rev-parse --verify "${COMMIT}^{commit}" 2>/dev/null)" || \
      die "提交不存在于 $repo: $COMMIT (可能需要先 fetch)"
    short="$(git -C "$repo" rev-parse --short "$rev")"
    sdir="$WORK/sources/$short"
    if [ ! -d "$sdir/.git" ] && [ ! -f "$sdir/.git" ]; then
      echo ">> 检出 $COMMIT 到 $sdir"
      git -C "$repo" worktree add --detach "$sdir" "$rev"
    fi
    BUILD_SRC="$sdir"
    BUILD_REV="$rev"
    BUILD_REPO="$repo"
  else
    BUILD_SRC="${src:?}"
    BUILD_REV="$(git -C "$BUILD_SRC" rev-parse HEAD 2>/dev/null || echo "unknown")"
    BUILD_REPO="$(git -C "$BUILD_SRC" rev-parse --show-toplevel 2>/dev/null || echo "$BUILD_SRC")"
  fi
  [ -d "$BUILD_SRC/os/StarryOS" ] || die "构建来源缺少 os/StarryOS: $BUILD_SRC"
}

# ---------- provision ----------

cmd_provision() {
  local img="${POS[0]:-}"
  [ -n "$img" ] && [ -f "$img" ] || die "provision 用法: sg2002-image-build.sh provision <官方Linux镜像.img>"
  resolve_assets_dir
  "$SCRIPTS/provision-sg2002.sh" "$img" "$WORK/assets"
  echo ">> 完成: fip.bin / ramdisk.bin 已就位 ($WORK/assets)"
}

# ---------- inject ----------

collect_inject_args() {
  INJECT_FLAT=()
  for i in "${INJECTS[@]}"; do INJECT_FLAT+=(--inject "$i"); done
  for m in "${MANIFESTS[@]}"; do INJECT_FLAT+=(--manifest "$m"); done
  if [ "$INIT_ASSETS" -eq 1 ]; then INJECT_FLAT+=(--init-assets); fi
}

cmd_inject() {
  resolve_assets_dir
  [ -f "$WORK/assets/rootfs.ext4" ] || die "资产缺少 rootfs.ext4, 先 build 或手动放置"
  collect_inject_args
  "$SCRIPTS/inject-rootfs.sh" "$WORK/assets/rootfs.ext4" ${INJECT_FLAT[@]+"${INJECT_FLAT[@]}"}
}

# ---------- build ----------

cmd_build() {
  check_deps
  resolve_assets_dir
  resolve_build_src

  # 1) 内核
  CONFIG="${CONFIG:-os/StarryOS/configs/board/licheerv-nano-sg2002.toml}"
  case "$CONFIG" in
    /*) CFG_ABS="$CONFIG" ;;
    *)  CFG_ABS="$BUILD_SRC/$CONFIG" ;;
  esac
  [ -f "$CFG_ABS" ] || die "板级配置不存在: $CFG_ABS"
  REL_CFG="${CFG_ABS#"$BUILD_SRC"/}"

  if [ -n "${KERNEL:-}" ]; then
    [ -f "$KERNEL" ] || die "内核不存在: $KERNEL"
    cp "$KERNEL" "$WORK/assets/starryos.bin"
    echo ">> 复用内核: $KERNEL"
    if [ -f "${KERNEL%.bin}.uimg" ]; then
      cp "${KERNEL%.bin}.uimg" "$WORK/assets/starryos.uimg"
    else
      warn "未找到 ${KERNEL%.bin}.uimg, 跳过 /starryos.uimg (仅影响手动引导回退)"
    fi
  else
    echo ">> 编译 starry: $BUILD_SRC  ($REL_CFG)"
    ( cd "$BUILD_SRC" && cargo xtask starry build -c "$REL_CFG" )
    KBIN="$BUILD_SRC/target/riscv64gc-unknown-none-elf/release/starryos.bin"
    [ -f "$KBIN" ] || die "编译产物不存在: $KBIN"
    cp "$KBIN" "$WORK/assets/starryos.bin"
    if [ -f "${KBIN%.bin}.uimg" ]; then
      cp "${KBIN%.bin}.uimg" "$WORK/assets/starryos.uimg"
    else
      warn "未生成 starryos.uimg (配置缺少 .its?), 跳过 /starryos.uimg"
    fi
  fi

  # 2) DTB
  if [ -n "${DTB:-}" ]; then
    [ -f "$DTB" ] || die "DTB 不存在: $DTB"
    cp "$DTB" "$WORK/assets/licheerv-nano-sg2002.dtb"
  else
    DTB_CAND="${CFG_ABS%.toml}.dtb"
    if [ -f "$DTB_CAND" ]; then
      cp "$DTB_CAND" "$WORK/assets/licheerv-nano-sg2002.dtb"
    elif [ -f "$BUILD_SRC/os/StarryOS/configs/board/licheerv-nano-sg2002.dtb" ]; then
      cp "$BUILD_SRC/os/StarryOS/configs/board/licheerv-nano-sg2002.dtb" "$WORK/assets/licheerv-nano-sg2002.dtb"
    else
      die "未找到 DTB: 用 --dtb 指定 (config 同名 .dtb 与默认 licheerv-nano-sg2002.dtb 均不存在)"
    fi
  fi
  echo ">> DTB: $WORK/assets/licheerv-nano-sg2002.dtb"

  # 3) rootfs
  RF="$BUILD_SRC/.tgos-images/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img"
  if [ -n "${ROOTFS:-}" ]; then
    [ -f "$ROOTFS" ] || die "rootfs 不存在: $ROOTFS"
    cp "$ROOTFS" "$WORK/assets/rootfs.ext4"
    echo ">> 复用 rootfs: $ROOTFS"
  elif [ -f "$WORK/assets/rootfs.ext4" ]; then
    echo ">> 复用工作目录 rootfs: $WORK/assets/rootfs.ext4 (重新下载请删除该文件)"
  elif [ -f "$RF" ]; then
    cp "$RF" "$WORK/assets/rootfs.ext4"
    echo ">> 复用已下载 rootfs: $RF"
  else
    echo ">> 下载 rootfs (Alpine riscv64)"
    ( cd "$BUILD_SRC" && cargo xtask starry rootfs --arch riscv64 )
    [ -f "$RF" ] || die "rootfs 下载产物不存在: $RF"
    cp "$RF" "$WORK/assets/rootfs.ext4"
  fi

  # 4) 注入用户资产与 init 套件
  collect_inject_args
  "$SCRIPTS/inject-rootfs.sh" "$WORK/assets/rootfs.ext4" ${INJECT_FLAT[@]+"${INJECT_FLAT[@]}"}

  # 5) /starryos.uimg (手动引导回退路径)
  if [ -f "$WORK/assets/starryos.uimg" ]; then
    "$SCRIPTS/inject-rootfs.sh" "$WORK/assets/rootfs.ext4" \
      --inject "$WORK/assets/starryos.uimg:/starryos.uimg"
  fi

  # 6) 打包 FIT
  ( cd "$WORK/assets" && ASSETS_DIR="$WORK/assets" ITS="$TEMPLATES/boot.its" "$SCRIPTS/repack-fit.sh" )

  # 7) 组装镜像
  OUT="${OUT:-$WORK/output/sg2002_starryos_$(date +%Y%m%d-%H%M%S).img}"
  ( cd "$WORK/assets" && FIP="$WORK/assets/fip.bin" FIT="$WORK/assets/boot.sd" \
      ROOTFS="$WORK/assets/rootfs.ext4" "$SCRIPTS/build-image.sh" "$OUT" )

  ln -sfn "$OUT" "$WORK/output/latest.img"
  write_buildinfo "$OUT"
  verify_image "$OUT"
  echo ">> 镜像: $OUT"
}

write_buildinfo() {
  local img="$1" json
  json="$(printf '{\n  "output": "%s",\n  "timestamp": "%s",\n  "repo": "%s",\n  "commit": "%s",\n  "config": "%s",\n  "kernel": "%s",\n  "rootfs": "%s",\n  "injects": [%s],\n  "init_assets": %s\n}\n' \
    "$img" "$(date '+%Y-%m-%d %H:%M:%S')" "$BUILD_REPO" "$BUILD_REV" "$CFG_ABS" \
    "$(basename "${KERNEL:-starryos.bin (built)}")" "$(basename "${ROOTFS:-rootfs.ext4}")" \
    "$(join_injects)" "$INIT_ASSETS")"
  echo "$json" > "$img.json"
  cp "$img.json" "$WORK/output/latest.json"
}

join_injects() {
  local list="" e
  for e in "${INJECTS[@]}"; do
    [ -n "$list" ] && list="$list, "
    list="$list\"$e\""
  done
  for m in "${MANIFESTS[@]}"; do
    [ -n "$list" ] && list="$list, "
    list="$list\"manifest: $m\""
  done
  echo "$list"
}

verify_image() {
  local img="$1"
  local BOOT_OFF P2_OFF tmp
  BOOT_OFF=$((2048 * 512))
  P2_OFF=$((133120 * 512))
  echo ">> 校验 $img"
  mdir -i "$img@@$BOOT_OFF" :: 2>/dev/null | grep -E "fip|boot" || true
  # boot.sd 内容: 从 FAT 提取后按 mkimage 头校验 (Load 地址与默认配置名)
  tmp="$(mktemp)"
  if mcopy -i "$img@@$BOOT_OFF" ::boot.sd "$tmp" 2>/dev/null; then
    if mkimage -l "$tmp" 2>/dev/null | grep -qE "Load Address:.*80200000"; then
      echo "  [ok] boot.sd Load=0x80200000"
    else
      warn "boot.sd Load 地址异常"
    fi
    if mkimage -l "$tmp" 2>/dev/null | grep -q "Default Configuration:.*config-sg2002_licheervnano_sd"; then
      echo "  [ok] boot.sd 默认配置 config-sg2002_licheervnano_sd"
    else
      warn "boot.sd 默认配置名异常"
    fi
  else
    warn "无法从 FAT 提取 boot.sd"
  fi
  rm -f "$tmp"
  if debugfs -R "stat /bin/sh" "$img?offset=$P2_OFF" 2>/dev/null | grep -q "Inode:"; then
    echo "  [ok] /bin/sh"
  else
    warn "无法读取镜像中的 /bin/sh"
  fi
  if debugfs -R "stat /starryos.uimg" "$img?offset=$P2_OFF" 2>/dev/null | grep -q "Inode:"; then
    echo "  [ok] /starryos.uimg (手动引导回退)"
  else
    warn "镜像中无 /starryos.uimg"
  fi
}

# p2 (ext4 rootfs) 分区字节偏移
p2_offset() {
  local img="$1" line
  line="$(sfdisk -d "$img" 2>/dev/null | grep -v 'label:' | grep 'type=83' | head -1)"
  [ -n "$line" ] || return 1
  echo $(( $(echo "$line" | grep -oP 'start=\s*\K\d+') * 512 ))
}

# update-* 的 buildinfo: 记录操作类型、基底镜像与来源信息 (与 build 的 json 字段互补)
write_buildinfo_update() {
  local img="$1" base="$2" op="$3" subject="$4" json repo rev
  if [ -f "$subject" ]; then
    repo="$(git -C "$(dirname "$subject")" rev-parse --show-toplevel 2>/dev/null || echo unknown)"
    rev="$(git -C "$(dirname "$subject")" log -1 --format='%h' 2>/dev/null || echo unknown)"
  else
    repo="unknown"; rev="unknown"
  fi
  json="$(printf '{\n  "output": "%s",\n  "timestamp": "%s",\n  "operation": "%s",\n  "base": "%s",\n  "subject": "%s",\n  "repo": "%s",\n  "commit": "%s",\n  "dtb": "%s",\n  "uimg": "%s",\n  "init_assets": %s\n}\n' \
    "$img" "$(date '+%Y-%m-%d %H:%M:%S')" "$op" "$(basename "$base")" "$(basename "$subject")" \
    "$repo" "$rev" "${DTB:-n/a}" "$([ "$SYNC_UIMG" -eq 1 ] && echo synced || echo skipped)" "$INIT_ASSETS")"
  echo "$json" > "$img.json"
  cp "$img.json" "$WORK/output/latest.json"
}

# ---------- update-kernel ----------

cmd_update_kernel() {
  local src="${POS[0]:-}"
  [ -n "$src" ] && [ -f "$src" ] || die "update-kernel 用法: sg2002-image-build.sh update-kernel <镜像.img> [选项]"
  resolve_assets_dir
  local kbin="${KERNEL:-$WORK/assets/starryos.bin}"
  [ -f "$kbin" ] || die "内核不存在: $kbin (先 build, 或 --kernel 指定)"

  # uimg 与内核配对: kbin 同目录的 uimg 同步到资产 (保证 assets 内 bin/uimg 一致)
  if [ -f "${kbin%.bin}.uimg" ] && \
     { [ ! -f "$WORK/assets/starryos.uimg" ] || ! cmp -s "${kbin%.bin}.uimg" "$WORK/assets/starryos.uimg"; }; then
    cp "${kbin%.bin}.uimg" "$WORK/assets/starryos.uimg"
    echo ">> 同步 uimg 资产: ${kbin%.bin}.uimg"
  fi

  local out
  if [ "$OVERWRITE" -eq 1 ]; then
    out="$src"
    echo ">> 就地更新内核: $src"
  else
    out="${OUT:-${src%.img}_kernel-$(date +%Y%m%d-%H%M%S).img}"
    echo ">> 复制 $src -> $out"
    # 不使用 reflink: WSL2 ext4 上 reflink 复制 2GB 镜像内容异常 (p2 读为全 0, 实测)
    cp --reflink=never "$src" "$out"
  fi

  ASSETS_DIR="$WORK/assets" "$SCRIPTS/swap-kernel.sh" "$out" "$kbin" "${DTB:-}"

  # /starryos.uimg 同步到镜像 rootfs (手动引导回退路径与 boot.sd 内核保持一致)
  if [ "$SYNC_UIMG" -eq 1 ] && [ -f "$WORK/assets/starryos.uimg" ]; then
    if p2="$(p2_offset "$out")" && [ -n "$p2" ]; then
      "$SCRIPTS/inject-rootfs.sh" "${out}?offset=${p2}" --inject "$WORK/assets/starryos.uimg:/starryos.uimg" >/dev/null
      echo ">> 已同步 /starryos.uimg 到镜像 rootfs"
    else
      warn "无法解析 p2 偏移, 跳过 /starryos.uimg 同步"
    fi
  fi

  write_buildinfo_update "$out" "$src" "update-kernel" "$kbin"
  ln -sfn "$out" "$WORK/output/latest.img"
  verify_image "$out"
  echo ">> 镜像: $out"
}

# ---------- update-rootfs ----------

cmd_update_rootfs() {
  local src="${POS[0]:-}"
  [ -n "$src" ] && [ -f "$src" ] || die "update-rootfs 用法: sg2002-image-build.sh update-rootfs <镜像.img> [选项]"
  resolve_assets_dir
  local rfs="${ROOTFS:-$WORK/assets/rootfs.ext4}"
  [ -f "$rfs" ] || die "rootfs 不存在: $rfs (先 build, 或 --rootfs 指定)"

  local out
  if [ "$OVERWRITE" -eq 1 ]; then
    out="$src"
    echo ">> 就地更新 rootfs: $src"
  else
    out="${OUT:-${src%.img}_rootfs-$(date +%Y%m%d-%H%M%S).img}"
    echo ">> 复制 $src -> $out"
    cp --reflink=never "$src" "$out"
  fi

  ASSETS_DIR="$WORK/assets" "$SCRIPTS/swap-rootfs.sh" "$out" "$rfs"
  write_buildinfo_update "$out" "$src" "update-rootfs" "$rfs"
  ln -sfn "$out" "$WORK/output/latest.img"
  verify_image "$out"
  echo ">> 镜像: $out"
}

# ---------- misc ----------

cmd_clean() {
  resolve_assets_dir
  rm -rf "$WORK/work"
  echo ">> 已清理 $WORK/work (assets/ 与 output/ 保留)"
  du -sh "$WORK"/* 2>/dev/null || true
}

case "$cmd" in
  provision)      cmd_provision ;;
  build)          cmd_build ;;
  update-kernel)  cmd_update_kernel ;;
  update-rootfs)  cmd_update_rootfs ;;
  inject)         cmd_inject ;;
  check-deps)     check_deps ;;
  clean)          cmd_clean ;;
esac
