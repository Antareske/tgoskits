#!/usr/bin/env bash
set -euo pipefail

SERIAL="${SERIAL:-/dev/ttyUSB0}"
printf 'reboot\n' > "$SERIAL"
