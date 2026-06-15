#!/usr/bin/env bash
# Deploy the staged install dir to the board's Linux rootfs at /act_infer_rk3588.
# StarryOS on RK3588 shares the board Linux rootfs, so the inference program and
# its assets are installed once on the Linux side and then reachable from Starry.
#
# Mirrors the deployment approach of apps/starry/orangepi-5-plus-uvc-rknn.
#
#   BOARD_IP=10.3.10.24 BOARD_USER=orangepi bash scripts/deploy-to-board.sh
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
install_dir="${app_dir}/install/rk3588_linux_aarch64/act_infer_rk3588"
board_ip="${BOARD_IP:?set BOARD_IP to the board address}"
board_user="${BOARD_USER:-orangepi}"
board_pass="${BOARD_PASS:-orangepi}"
dest="/act_infer_rk3588"

if [[ ! -d "${install_dir}" ]]; then
    echo "error: ${install_dir} not found; run scripts/build-rk3588.sh first" >&2
    exit 1
fi

echo "info: syncing ${install_dir}/ -> ${board_user}@${board_ip}:/tmp/act_infer_rk3588/"
rsync -az --delete "${install_dir}/" "${board_user}@${board_ip}:/tmp/act_infer_rk3588/"

echo "info: installing to ${dest} (requires sudo on board)"
ssh "${board_user}@${board_ip}" "
  printf '%s\n' '${board_pass}' | sudo -S rm -rf ${dest} &&
  printf '%s\n' '${board_pass}' | sudo -S mv /tmp/act_infer_rk3588 ${dest} &&
  printf '%s\n' '${board_pass}' | sudo -S chown -R root:root ${dest} &&
  sync
"
echo "info: deployed to ${board_user}@${board_ip}:${dest}"
echo "info: board Linux smoke test:"
echo "  ssh ${board_user}@${board_ip} 'sudo ${dest}/run-review.sh left'"
