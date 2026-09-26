#!/usr/bin/env bash
# inject-rootfs.sh - 向 ext4 rootfs 注入文件 (debugfs, 无 loop 设备场景)
#
# 目标已存在时先删后写; 缺失的父目录逐级探测并自动创建;
# mode 为 4 位八进制 (如 0755), 通过 set_inode_field 设置。
#
# 用法:
#   inject-rootfs.sh <rootfs.ext4> [--inject host:target[:mode]]... \
#                                  [--manifest file]... [--init-assets]
# host 路径相对当前目录; manifest 中的 host 路径相对 manifest 文件所在目录。
#
# 除独立 rootfs.ext4 外, 也支持直接对整盘镜像的 rootfs 分区注入,
# 传入 debugfs 的偏移语法即可 (如 "<镜像.img>?offset=68157440"), 用于
# update-kernel 后同步 /starryos.uimg 等场景。
set -euo pipefail
SKILL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ROOTFS="${1:?用法: inject-rootfs.sh <rootfs.ext4> [--inject host:target[:mode]]...}"; shift
# 镜像分区路径 (含 ?offset=) 不是普通文件, 跳过文件存在性检查
case "$ROOTFS" in
  *'?offset='*) ;;
  *) [ -f "$ROOTFS" ] || { echo "rootfs 不存在: $ROOTFS" >&2; exit 1; } ;;
esac

die() { echo "$*" >&2; exit 1; }

# debugfs 路径探测: 存在 (含目录/文件/符号链接) 返回 0
df_exists() {
  debugfs -R "stat $1" "$ROOTFS" 2>/dev/null | grep -q "^Inode:"
}

# 逐级创建目录 (对已存在的组件跳过)
df_mkdirp() {
  local dir="$1" cur="" part
  [ "$dir" = "/" ] && return 0
  while IFS= read -r part; do
    [ -z "$part" ] && continue
    cur="$cur/$part"
    df_exists "$cur" || debugfs -w -R "mkdir $cur" "$ROOTFS" >/dev/null
  done <<< "$(echo "${dir#/}" | tr '/' '\n')"
}

# 写文件: host -> target, 可选 mode (4 位八进制, 自动补 S_IFREG 类型位)
df_write() {
  local host="$1" target="$2" mode="${3:-}" full
  [ -f "$host" ] || die "注入文件不存在: $host"
  case "$target" in /*) ;; *) die "target 必须是 rootfs 内绝对路径: $target" ;; esac
  df_mkdirp "$(dirname "$target")"
  df_exists "$target" && debugfs -w -R "rm $target" "$ROOTFS" >/dev/null
  debugfs -w -R "write $host $target" "$ROOTFS" >/dev/null
  if [ -n "$mode" ]; then
    # set_inode_field mode 直接覆盖整个 mode (含类型位), 需补 S_IFREG(0100)
    case "${#mode}" in
      4) full="0100${mode#0}" ;;
      5) full="$mode" ;;
      *) die "mode 需为 4 位八进制 (如 0755): $mode" ;;
    esac
    debugfs -w -R "set_inode_field $target mode $full" "$ROOTFS" >/dev/null
  fi
  echo "  [inject] $host -> $target${mode:+ ($mode)}"
}

# 内置 init 套件: starry-init.sh + sshd_config + inittab + /etc/profile 安全网
init_assets() {
  local A="$SKILL_ROOT/templates/init-assets"
  df_write "$A/starry-init.sh" /usr/bin/starry-init.sh 0755
  df_write "$A/sshd_config" /etc/ssh/sshd_config
  df_write "$A/inittab" /etc/inittab

  # /etc/profile 追加调用 (幂等, inittab 失败时的回退路径)
  if df_exists /etc/profile; then
    local tmp
    tmp="$(mktemp)"
    debugfs -R "cat /etc/profile" "$ROOTFS" 2>/dev/null > "$tmp" || true
    if ! grep -q '/usr/bin/starry-init.sh' "$tmp"; then
      echo "/usr/bin/starry-init.sh" >> "$tmp"
      debugfs -w -R "rm /etc/profile" "$ROOTFS" >/dev/null
      debugfs -w -R "write $tmp /etc/profile" "$ROOTFS" >/dev/null
      echo "  [inject] /etc/profile += /usr/bin/starry-init.sh"
    fi
    rm -f "$tmp"
  fi

  # /var/empty 所有权 (sshd 要求 root)
  if df_exists /var/empty; then
    debugfs -w -R "set_inode_field /var/empty uid 0" "$ROOTFS" >/dev/null
    debugfs -w -R "set_inode_field /var/empty gid 0" "$ROOTFS" >/dev/null
    echo "  [inject] /var/empty uid/gid -> 0:0"
  fi
}

ENTRIES=()   # 元素: host|target|mode
INIT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --inject)
      spec="${2:?--inject 需要 host:target[:mode]}"
      IFS=: read -r h t m <<< "$spec"
      [ -n "$h" ] && [ -n "$t" ] || die "--inject 格式: host:target[:mode], 得到: $spec"
      ENTRIES+=("$h|$t|$m")
      shift 2 ;;
    --manifest)
      mf="${2:?--manifest 需要文件路径}"
      [ -f "$mf" ] || die "manifest 不存在: $mf"
      mdir="$(cd "$(dirname "$mf")" && pwd)"
      while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in ''|'#'*) continue ;; esac
        read -r h t m <<< "$line"
        [ -n "$h" ] && [ -n "$t" ] || die "manifest 行格式: <host> <target> [mode], 得到: $line"
        case "$h" in /*) hp="$h" ;; *) hp="$mdir/$h" ;; esac
        ENTRIES+=("$hp|$t|$m")
      done < "$mf"
      shift 2 ;;
    --init-assets)
      INIT=1
      shift ;;
    *)
      die "未知参数: $1" ;;
  esac
done

for e in "${ENTRIES[@]}"; do
  IFS='|' read -r h t m <<< "$e"
  df_write "$h" "$t" "$m"
done

if [ "$INIT" -eq 1 ]; then init_assets; fi

echo ">> 注入完成: $ROOTFS"
